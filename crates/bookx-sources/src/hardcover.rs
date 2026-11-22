//! Hardcover (https://hardcover.app) reading lists, read only: the books on the user's shelves and
//! their status (want to read, reading, read, did not finish) are fetched through the GraphQL API
//! with the user's API token and matched against the library by ISBN, then by title and author.
//! Nothing is ever written to Hardcover and nothing is applied here: [`plan`] only says what
//! would change, the caller decides.
//!
//! The server address is fixed (`api.hardcover.app`), the token travels only in the
//! `Authorization` header and is never part of an error text. Written against the public schema
//! (`me.user_books`); checked here against a stub server, not against the live service.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::{Value, json};

use crate::fetch::{FetchError, Fetcher};
use crate::isbn;
use crate::text::normalize;

pub const ENDPOINT: &str = "https://api.hardcover.app/v1/graphql";
pub const PAGE: usize = 100;
/// Most pages read in one sync (10 000 books).
pub const MAX_PAGES: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    WantToRead,
    Reading,
    Read,
    DidNotFinish,
}

impl Status {
    pub fn from_id(id: u64) -> Option<Status> {
        Some(match id {
            1 => Status::WantToRead,
            2 => Status::Reading,
            3 => Status::Read,
            5 => Status::DidNotFinish,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub status: Status,
    pub title: String,
    pub authors: Vec<String>,
    pub isbns: Vec<String>,
    /// The user's rating on the Calibre scale (0 to 10, two per star); `None` when not rated.
    pub rating: Option<u8>,
}

const QUERY: &str = "query($limit:Int!,$offset:Int!){me{user_books(limit:$limit,offset:$offset,order_by:{id:asc}){status_id rating book{title contributions{author{name}}} edition{isbn_10 isbn_13}}}}";

/// The JSON body of one page request.
pub fn request_body(page: usize) -> String {
    json!({"query": QUERY, "variables": {"limit": PAGE, "offset": page * PAGE}}).to_string()
}

fn auth_header(token: &str) -> String {
    let t = token.trim();
    if t.to_ascii_lowercase().starts_with("bearer ") {
        t.to_string()
    } else {
        format!("Bearer {t}")
    }
}

/// Reads one answer: the items and whether the page was full (so there may be more).
pub fn parse_page(body: &[u8]) -> Result<(Vec<Item>, bool)> {
    let v: Value = serde_json::from_slice(body).context("Hardcover did not answer with JSON")?;
    if let Some(m) = v.pointer("/errors/0/message").and_then(Value::as_str) {
        bail!("Hardcover said: {m}");
    }
    // `me` is a list in the public schema; accept a single object too
    let me = v
        .pointer("/data/me")
        .ok_or_else(|| anyhow!("Hardcover sent no list"))?;
    let rows = match me {
        Value::Array(a) => a.first().cloned().unwrap_or(Value::Null),
        other => other.clone(),
    };
    let list = rows
        .get("user_books")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Hardcover sent no list of books"))?;
    let mut out = Vec::new();
    for ub in list {
        let Some(status) = ub
            .get("status_id")
            .and_then(Value::as_u64)
            .and_then(Status::from_id)
        else {
            continue;
        };
        let title = ub
            .pointer("/book/title")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if title.is_empty() {
            continue;
        }
        let authors = ub
            .pointer("/book/contributions")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|c| c.pointer("/author/name").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let isbns = ["isbn_13", "isbn_10"]
            .iter()
            .filter_map(|k| ub.pointer(&format!("/edition/{k}")).and_then(Value::as_str))
            .map(isbn::normalize)
            .filter(|s| !s.is_empty())
            .collect();
        let rating = ub
            .get("rating")
            .and_then(Value::as_f64)
            .map(|r| (r * 2.0).round().clamp(0.0, 10.0) as u8)
            .filter(|r| *r > 0);
        out.push(Item {
            status,
            title,
            authors,
            isbns,
            rating,
        });
    }
    Ok((out, list.len() >= PAGE))
}

/// Asks for one page only: whether the token works, and how many books that page holds (the page
/// size when there may be more). The token never appears in the error.
pub fn test_connection(fetcher: &dyn Fetcher, token: &str) -> Result<usize> {
    if token.trim().is_empty() {
        bail!("Paste your Hardcover API token first.");
    }
    let headers = [
        ("Authorization".to_string(), auth_header(token)),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    let body = fetcher
        .post(
            ENDPOINT,
            &headers,
            "application/json",
            request_body(0).as_bytes(),
        )
        .map_err(|e| match FetchError::of(&e) {
            Some(FetchError::Http(401 | 403)) => {
                anyhow!(
                    "Hardcover did not accept the token: copy a new one from your account settings"
                )
            }
            _ => e.context("Could not reach Hardcover"),
        })?;
    Ok(parse_page(&body)?.0.len())
}

/// Reads the shelves of the token's owner.
pub fn fetch_items(fetcher: &dyn Fetcher, token: &str) -> Result<Vec<Item>> {
    if token.trim().is_empty() {
        bail!("Paste your Hardcover API token first.");
    }
    let headers = [
        ("Authorization".to_string(), auth_header(token)),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    let mut all = Vec::new();
    for page in 0..MAX_PAGES {
        let body = fetcher
            .post(ENDPOINT, &headers, "application/json", request_body(page).as_bytes())
            .map_err(|e| match FetchError::of(&e) {
                Some(FetchError::Http(401 | 403)) => {
                    anyhow!("Hardcover did not accept the token: copy a new one from your account settings")
                }
                _ => e.context("Could not read your lists from Hardcover"),
            })?;
        let (items, more) = parse_page(&body)?;
        all.extend(items);
        if !more {
            break;
        }
    }
    Ok(all)
}

/// A book of the library as far as matching needs it.
#[derive(Debug, Clone, PartialEq)]
pub struct Local {
    pub id: u64,
    pub title: String,
    pub authors: Vec<String>,
    pub isbns: Vec<String>,
}

fn same_title_author(l: &Local, i: &Item) -> bool {
    if normalize(&l.title) != normalize(&i.title) {
        return false;
    }
    // without an author on either side the title has to do
    if l.authors.is_empty() || i.authors.is_empty() {
        return true;
    }
    let last = |a: &str| {
        normalize(a)
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_string()
    };
    l.authors.iter().any(|a| {
        i.authors
            .iter()
            .any(|b| !last(a).is_empty() && last(a) == last(b))
    })
}

/// Which library book each Hardcover item is: by ISBN first, then by title and author. A book is
/// matched at most once; an item that matches several books is left alone (no guessing).
pub fn plan(items: &[Item], local: &[Local]) -> Vec<(u64, Status)> {
    match_items(items, local)
        .into_iter()
        .map(|(id, i)| (id, items[i].status))
        .collect()
}

/// Like [`plan`], but gives the index of the matched item (so the caller can read its rating).
pub fn match_items(items: &[Item], local: &[Local]) -> Vec<(u64, usize)> {
    let mut out: Vec<(u64, usize)> = Vec::new();
    for (index, i) in items.iter().enumerate() {
        let by_isbn: Vec<&Local> = local
            .iter()
            .filter(|l| {
                l.isbns
                    .iter()
                    .any(|x| i.isbns.contains(&isbn::normalize(x)))
            })
            .collect();
        let hits = if by_isbn.len() == 1 {
            by_isbn
        } else {
            local.iter().filter(|l| same_title_author(l, i)).collect()
        };
        if let [one] = hits.as_slice() {
            if !out.iter().any(|(id, _)| *id == one.id) {
                out.push((one.id, index));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};

    fn page(n: usize, start: usize) -> String {
        let rows: Vec<String> = (start..start + n)
            .map(|i| {
                format!(
                    r#"{{"status_id":3,"book":{{"title":"Book {i}","contributions":[{{"author":{{"name":"Ann Writer"}}}}]}},"edition":{{"isbn_13":"978-0-306-40615-7","isbn_10":null}}}}"#
                )
            })
            .collect();
        format!(
            r#"{{"data":{{"me":[{{"user_books":[{}]}}]}}}}"#,
            rows.join(",")
        )
    }

    #[test]
    fn a_page_is_read() {
        let (items, more) = parse_page(page(2, 0).as_bytes()).unwrap();
        assert!(!more);
        assert_eq!(items[0].title, "Book 0");
        assert_eq!(items[0].status, Status::Read);
        assert_eq!(items[0].authors, vec!["Ann Writer"]);
        assert_eq!(items[0].isbns, vec!["9780306406157"]);
        // unknown statuses and nameless books are left out
        let odd = r#"{"data":{"me":{"user_books":[{"status_id":9,"book":{"title":"X"}},{"status_id":1,"book":{"title":" "}},{"status_id":2,"book":{"title":"Y"}}]}}}"#;
        let (items, _) = parse_page(odd.as_bytes()).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].status, Status::Reading);
        assert!(
            parse_page(br#"{"errors":[{"message":"Invalid token"}]}"#)
                .unwrap_err()
                .to_string()
                .contains("Invalid token")
        );
        assert!(parse_page(b"nope").is_err());
        assert!(parse_page(b"{}").is_err());
    }

    #[test]
    fn the_token_goes_in_the_header_only_and_pages_are_followed() {
        let f = MockFetcher::new().with_body(ENDPOINT, page(PAGE, 0));
        // every page is full: the walk ends at the page limit and never loops
        let items = fetch_items(&f, "abc123").unwrap();
        assert_eq!(items.len(), PAGE * MAX_PAGES);
        let posts = f.posts();
        assert_eq!(posts.len(), MAX_PAGES);
        assert_eq!(
            posts[0].0.header_value("authorization"),
            Some("Bearer abc123")
        );
        assert!(!String::from_utf8_lossy(&posts[0].1).contains("abc123"));
        assert!(String::from_utf8_lossy(&posts[1].1).contains("\"offset\":100"));
        // a token that already says Bearer is not doubled
        let g = MockFetcher::new().with_body(ENDPOINT, page(1, 0));
        fetch_items(&g, "Bearer xyz").unwrap();
        assert_eq!(
            g.posts()[0].0.header_value("authorization"),
            Some("Bearer xyz")
        );
        assert_eq!(g.posts().len(), 1, "a short page is the last");
    }

    #[test]
    fn errors_name_the_problem_and_never_the_token() {
        let f = MockFetcher::new().on(ENDPOINT, MockReply::Status(401));
        let e = format!("{:#}", fetch_items(&f, "secret-token").unwrap_err());
        assert!(
            e.contains("did not accept the token") && !e.contains("secret-token"),
            "{e}"
        );
        let f = MockFetcher::new().on(ENDPOINT, MockReply::Status(500));
        let e = format!("{:#}", fetch_items(&f, "secret-token").unwrap_err());
        assert!(!e.contains("secret-token"), "{e}");
        assert!(fetch_items(&f, "  ").is_err());
        assert!(f.posts().is_empty() || f.posts().len() == 1);
    }

    fn local(id: u64, title: &str, authors: &[&str], isbns: &[&str]) -> Local {
        Local {
            id,
            title: title.into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            isbns: isbns.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn item(title: &str, authors: &[&str], isbns: &[&str], status: Status) -> Item {
        Item {
            status,
            title: title.into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            isbns: isbns.iter().map(|s| s.to_string()).collect(),
            rating: None,
        }
    }

    #[test]
    fn ratings_become_calibre_stars_and_a_test_reads_one_page() {
        let body = r#"{"data":{"me":[{"user_books":[{"status_id":3,"rating":4.5,"book":{"title":"A"}},{"status_id":3,"rating":0,"book":{"title":"B"}},{"status_id":3,"rating":null,"book":{"title":"C"}}]}]}}"#;
        let (items, _) = parse_page(body.as_bytes()).unwrap();
        assert_eq!(
            items.iter().map(|i| i.rating).collect::<Vec<_>>(),
            vec![Some(9), None, None]
        );
        let f = MockFetcher::new().with_body(ENDPOINT, body);
        assert_eq!(test_connection(&f, "tok").unwrap(), 3);
        assert_eq!(f.posts().len(), 1);
        let f = MockFetcher::new().on(ENDPOINT, MockReply::Status(401));
        let e = format!("{:#}", test_connection(&f, "sentinel-token").unwrap_err());
        assert!(e.contains("did not accept") && !e.contains("sentinel-token"));
        assert!(test_connection(&f, " ").is_err());
    }

    #[test]
    fn items_are_matched_by_isbn_then_title_and_author() {
        let books = vec![
            local(1, "Dune", &["Frank Herbert"], &["978-0-441-17271-9"]),
            local(2, "The Left Hand of Darkness", &["Ursula K. Le Guin"], &[]),
            local(3, "Dune", &["Someone Else"], &[]),
        ];
        let items = vec![
            item("Dune (Deluxe)", &[], &["9780441172719"], Status::Read),
            item(
                "The left hand of darkness",
                &["Ursula Le Guin"],
                &[],
                Status::Reading,
            ),
            item("Dune", &["Frank Herbert"], &[], Status::WantToRead),
            item("Unknown", &["X"], &[], Status::Read),
        ];
        let p = plan(&items, &books);
        // ISBN wins for book 1, title+author for book 2; the third item would hit book 1 again
        // (already matched) and book 3 has another author: nothing is guessed
        assert_eq!(p, vec![(1, Status::Read), (2, Status::Reading)]);
        // two equally good books: no guess
        let twins = vec![local(1, "Same", &[], &[]), local(2, "Same", &[], &[])];
        assert!(plan(&[item("Same", &[], &[], Status::Read)], &twins).is_empty());
    }
}
