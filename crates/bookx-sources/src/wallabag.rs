//! Wallabag (P19): the saved articles of a Wallabag server (https://wallabag.org) become books.
//!
//! Wallabag has already fetched and cleaned each page, so the text comes from its API
//! (`GET /api/entries.json`), not from the original site; only the pictures are downloaded,
//! through the same guarded [`Fetcher`] as every other web import. The server address is something
//! the user typed, so its host may be on the local network (it is trusted for that one host); the
//! pictures and links inside the articles are not.
//!
//! Signing in uses the OAuth password grant of Wallabag (`POST /oauth/v2/token` with the API
//! client of the user's account), or the user pastes an access token. Nothing here stores
//! anything: the caller keeps the secrets (the keyring) and the list of entries already imported.

use anyhow::{Context, Result, anyhow, bail};
use serde_json::Value;

use crate::fetch::{FetchError, Fetcher, url_encode};
use crate::readability::{Article, clean_fragment};
use crate::sources::normalize_date;

/// Most pages asked for in one run (with 30 entries each).
pub const MAX_PAGES: usize = 20;
/// Entries asked for per page.
pub const PER_PAGE: usize = 30;
/// Most entries one run returns.
pub const MAX_ENTRIES: usize = 200;

/// The address of the server without a path: `wallabag.example.org/` and
/// `https://wallabag.example.org/api` both become `https://wallabag.example.org`. A server in a
/// sub-folder (`https://example.org/wallabag`) keeps it.
pub fn normalize_base(input: &str) -> Result<String> {
    let t = input.trim();
    if t.is_empty() {
        bail!("Enter the address of your Wallabag server.");
    }
    if t.contains(char::is_whitespace) {
        bail!("The address must not contain spaces.");
    }
    let with = if t.contains("://") {
        t.to_string()
    } else {
        format!("https://{t}")
    };
    let u = url::Url::parse(&with).with_context(|| format!("`{t}` is not a valid address"))?;
    if !matches!(u.scheme(), "http" | "https") {
        bail!("Only http and https addresses can be used.");
    }
    if u.host_str().is_none_or(str::is_empty) {
        bail!("`{t}` has no host name.");
    }
    if !u.username().is_empty() || u.password().is_some() {
        bail!("Do not put a user name or password in the address.");
    }
    let mut path = u.path().trim_end_matches('/').to_string();
    for tail in ["/api", "/api/entries", "/api/entries.json"] {
        if let Some(p) = path.strip_suffix(tail) {
            path = p.to_string();
            break;
        }
    }
    let mut out = format!("{}://{}", u.scheme(), u.host_str().unwrap_or(""));
    if let Some(p) = u.port() {
        out.push_str(&format!(":{p}"));
    }
    out.push_str(&path);
    Ok(out)
}

/// The API client and the account of the user (all four are needed for the password grant).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Credentials {
    pub client_id: String,
    pub client_secret: String,
    pub username: String,
    pub password: String,
}

impl Credentials {
    pub fn complete(&self) -> bool {
        [
            &self.client_id,
            &self.client_secret,
            &self.username,
            &self.password,
        ]
        .iter()
        .all(|s| !s.trim().is_empty())
    }
}

/// The form body of the token request.
pub fn token_form(c: &Credentials) -> String {
    format!(
        "grant_type=password&client_id={}&client_secret={}&username={}&password={}",
        url_encode(c.client_id.trim()),
        url_encode(c.client_secret.trim()),
        url_encode(c.username.trim()),
        url_encode(&c.password)
    )
}

/// Reads the access token out of the answer of `/oauth/v2/token`.
pub fn parse_token(body: &[u8]) -> Result<String> {
    let v: Value = serde_json::from_slice(body).context("Wallabag did not answer with JSON")?;
    if let Some(e) = v.get("error_description").and_then(Value::as_str) {
        bail!("Wallabag refused the sign-in: {e}");
    }
    v.get("access_token")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
        .ok_or_else(|| anyhow!("Wallabag sent no access token"))
}

/// Signs in with the API client and the account.
pub fn sign_in(fetcher: &dyn Fetcher, base: &str, c: &Credentials) -> Result<String> {
    if !c.complete() {
        bail!(
            "Fill in the client ID, client secret, user name and password, or paste an access token."
        );
    }
    crate::netguard::trust_url(base);
    let body = fetcher
        .post(
            &format!("{base}/oauth/v2/token"),
            &[("Accept".into(), "application/json".into())],
            "application/x-www-form-urlencoded",
            token_form(c).as_bytes(),
        )
        .map_err(|e| match FetchError::of(&e) {
            Some(FetchError::Http(400 | 401)) => {
                anyhow!("Wallabag refused the sign-in: check the client ID, secret, user name and password")
            }
            _ => e.context("Could not sign in to Wallabag"),
        })?;
    parse_token(&body)
}

/// One saved article.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Entry {
    pub id: u64,
    pub url: String,
    pub title: String,
    /// The cleaned page as HTML.
    pub content: String,
    pub language: String,
    pub authors: Vec<String>,
    pub published: String,
    pub preview_picture: Option<String>,
    pub tags: Vec<String>,
    pub archived: bool,
    pub starred: bool,
}

/// What to ask for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Query {
    /// Only the articles not yet archived (read) in Wallabag.
    pub unread_only: bool,
    pub starred_only: bool,
    /// Only articles with this tag.
    pub tag: Option<String>,
    /// Most entries to return (at most [`MAX_ENTRIES`]).
    pub limit: usize,
}

impl Default for Query {
    fn default() -> Self {
        Query {
            unread_only: true,
            starred_only: false,
            tag: None,
            limit: 25,
        }
    }
}

/// The address of one page of the list.
pub fn entries_url(base: &str, q: &Query, page: usize) -> String {
    let mut u = format!(
        "{base}/api/entries.json?perPage={PER_PAGE}&page={page}&sort=created&order=desc&detail=full"
    );
    if q.unread_only {
        u.push_str("&archive=0");
    }
    if q.starred_only {
        u.push_str("&starred=1");
    }
    if let Some(t) = q.tag.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
        u.push_str(&format!("&tags={}", url_encode(t)));
    }
    u
}

/// Reads one page of the list: `(entries, number of pages)`. Entries without an id, an address or
/// text are left out; only http(s) addresses are kept.
pub fn parse_page(body: &[u8]) -> Result<(Vec<Entry>, usize)> {
    let v: Value = serde_json::from_slice(body).context("Wallabag did not answer with JSON")?;
    if let Some(e) = v.get("error_description").and_then(Value::as_str) {
        bail!("Wallabag said: {e}");
    }
    let pages = v.get("pages").and_then(Value::as_u64).unwrap_or(1) as usize;
    let items = v
        .pointer("/_embedded/items")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow!("Wallabag sent no list of articles"))?;
    let text = |x: &Value, k: &str| {
        x.get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let mut out = Vec::new();
    for x in items {
        let Some(id) = x.get("id").and_then(Value::as_u64) else {
            continue;
        };
        let url = text(x, "url");
        let content = text(x, "content");
        if content.is_empty() || !(url.starts_with("http://") || url.starts_with("https://")) {
            continue;
        }
        let picture = text(x, "preview_picture");
        out.push(Entry {
            id,
            title: text(x, "title"),
            content,
            language: text(x, "language"),
            authors: x
                .get("published_by")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            published: {
                let p = text(x, "published_at");
                normalize_date(
                    if p.is_empty() {
                        text(x, "created_at")
                    } else {
                        p
                    }
                    .as_str(),
                )
            },
            preview_picture: (picture.starts_with("http://") || picture.starts_with("https://"))
                .then_some(picture),
            tags: x
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|t| t.get("label").and_then(Value::as_str))
                        .map(String::from)
                        .collect()
                })
                .unwrap_or_default(),
            archived: x.get("is_archived").and_then(Value::as_u64).unwrap_or(0) != 0,
            starred: x.get("is_starred").and_then(Value::as_u64).unwrap_or(0) != 0,
            url,
        });
    }
    Ok((out, pages))
}

/// Downloads the newest entries that `skip` does not refuse (the ones imported before), up to the
/// limit of `q`, paging through the list.
pub fn fetch_entries(
    fetcher: &dyn Fetcher,
    base: &str,
    token: &str,
    q: &Query,
    skip: &dyn Fn(u64) -> bool,
) -> Result<Vec<Entry>> {
    crate::netguard::trust_url(base);
    let limit = q.limit.clamp(1, MAX_ENTRIES);
    let headers = [
        ("Authorization".to_string(), format!("Bearer {token}")),
        ("Accept".to_string(), "application/json".to_string()),
    ];
    let mut out: Vec<Entry> = Vec::new();
    let mut page = 1;
    while page <= MAX_PAGES && out.len() < limit {
        let body = fetcher
            .get(&entries_url(base, q, page), &headers)
            .map_err(|e| match FetchError::of(&e) {
                Some(FetchError::Http(401 | 403)) => anyhow!(
                    "Wallabag did not accept the access token: sign in again or paste a new token"
                ),
                Some(FetchError::Http(404)) => {
                    anyhow!("There is no Wallabag API at {base}: check the address")
                }
                _ => e.context("Could not read the articles from Wallabag"),
            })?;
        let (entries, pages) = parse_page(&body)?;
        for e in entries {
            if out.len() >= limit {
                break;
            }
            if !skip(e.id) && !out.iter().any(|o| o.id == e.id) {
                out.push(e);
            }
        }
        if page >= pages {
            break;
        }
        page += 1;
    }
    Ok(out)
}

/// The entry as an [`Article`]: Wallabag's HTML goes through the same whitelist as every web page
/// (scripts, styles, forms and the like are dropped), relative links resolve against the original
/// address.
pub fn entry_article(e: &Entry) -> Article {
    let frag = clean_fragment(&e.content, &e.url);
    Article {
        url: e.url.clone(),
        title: e.title.clone(),
        author: e.authors.join(", "),
        site: crate::netguard::host_of(&e.url).unwrap_or_default(),
        published: e.published.clone(),
        description: String::new(),
        language: e.language.clone(),
        lead_image: e.preview_picture.clone(),
        content: frag.content,
        images: frag.images,
        text_len: frag.text_len,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{MockFetcher, MockReply};

    fn page_json(ids: &[u64], page: usize, pages: usize) -> String {
        let items: Vec<String> = ids
            .iter()
            .map(|id| {
                format!(
                    r#"{{"id":{id},"url":"https://blog.example/post-{id}","title":"Post {id}",
                    "content":"<p>Text of post {id}.</p><script>alert(1)</script><img src=\"/p{id}.png\">",
                    "language":"en","published_by":["Pat Writer","Sam"],"published_at":"2024-05-0{}T10:00:00+0000",
                    "created_at":"2024-06-01T10:00:00+0000","preview_picture":"https://blog.example/lead{id}.jpg",
                    "tags":[{{"id":1,"label":"rust","slug":"rust"}}],"is_archived":0,"is_starred":1}}"#,
                    id % 9
                )
            })
            .collect();
        format!(
            r#"{{"page":{page},"limit":30,"pages":{pages},"total":99,"_embedded":{{"items":[{}]}}}}"#,
            items.join(",")
        )
    }

    #[test]
    fn addresses_are_normalised() {
        for (i, o) in [
            ("wallabag.example.org", "https://wallabag.example.org"),
            (" https://w.example.org/ ", "https://w.example.org"),
            ("https://w.example.org/api", "https://w.example.org"),
            (
                "http://192.168.1.5:8080/wallabag/api/entries.json",
                "http://192.168.1.5:8080/wallabag",
            ),
            ("https://w.example.org/?x=1#y", "https://w.example.org"),
        ] {
            assert_eq!(normalize_base(i).unwrap(), o, "{i}");
        }
        for bad in [
            "",
            "two words",
            "ftp://x.org",
            "https://",
            "https://user:pw@w.example.org",
            "file:///etc",
        ] {
            assert!(normalize_base(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_token_request_is_a_correctly_encoded_form() {
        let c = Credentials {
            client_id: "1_abc".into(),
            client_secret: "s&e=cret".into(),
            username: "pat".into(),
            password: "p w+ä".into(),
        };
        assert_eq!(
            token_form(&c),
            "grant_type=password&client_id=1_abc&client_secret=s%26e%3Dcret&username=pat&password=p%20w%2B%C3%A4"
        );
    }

    #[test]
    fn signing_in_posts_the_form_and_reads_the_token() {
        let f = MockFetcher::new().with_body(
            "https://w.example.org/oauth/v2/token",
            r#"{"access_token":"TOK","expires_in":3600,"token_type":"bearer"}"#,
        );
        let c = Credentials {
            client_id: "i".into(),
            client_secret: "s".into(),
            username: "u".into(),
            password: "p".into(),
        };
        assert_eq!(sign_in(&f, "https://w.example.org", &c).unwrap(), "TOK");
        let posts = f.posts();
        assert_eq!(posts.len(), 1);
        assert_eq!(
            posts[0].0.header_value("content-type"),
            Some("application/x-www-form-urlencoded")
        );
        assert!(String::from_utf8_lossy(&posts[0].1).contains("grant_type=password"));
        // wrong password: the server answers 400, the message names what to check
        let bad = MockFetcher::new().on(
            "https://w.example.org/oauth/v2/token",
            MockReply::Status(400),
        );
        let e = sign_in(&bad, "https://w.example.org", &c).unwrap_err();
        assert!(format!("{e:#}").contains("refused the sign-in"), "{e:#}");
        // missing fields never reach the network
        let none = MockFetcher::new();
        assert!(sign_in(&none, "https://w.example.org", &Credentials::default()).is_err());
        assert!(none.posts().is_empty());
        // an error document inside a 200 answer
        assert!(
            parse_token(
                br#"{"error":"invalid_grant","error_description":"Invalid username and password"}"#
            )
            .unwrap_err()
            .to_string()
            .contains("Invalid username")
        );
        assert!(parse_token(b"<html>").is_err());
        assert!(parse_token(br#"{"access_token":""}"#).is_err());
    }

    #[test]
    fn a_page_of_entries_is_read() {
        let (e, pages) = parse_page(page_json(&[7], 1, 4).as_bytes()).unwrap();
        assert_eq!(pages, 4);
        assert_eq!(e.len(), 1);
        let e = &e[0];
        assert_eq!((e.id, e.title.as_str()), (7, "Post 7"));
        assert_eq!(e.authors, vec!["Pat Writer", "Sam"]);
        assert_eq!(e.published, "2024-05-07");
        assert_eq!(e.tags, vec!["rust"]);
        assert!(e.starred && !e.archived);
        // entries that cannot become a book are left out
        let odd = r#"{"pages":1,"_embedded":{"items":[
            {"id":1,"url":"javascript:alert(1)","content":"<p>x</p>"},
            {"id":2,"url":"https://a.example/x","content":""},
            {"url":"https://a.example/y","content":"<p>no id</p>"},
            {"id":3,"url":"https://a.example/z","content":"<p>ok</p>"}]}}"#;
        let (e, _) = parse_page(odd.as_bytes()).unwrap();
        assert_eq!(e.iter().map(|x| x.id).collect::<Vec<_>>(), vec![3]);
        assert!(parse_page(b"{}").is_err());
        assert!(parse_page(b"nope").is_err());
    }

    #[test]
    fn entries_are_collected_over_pages_skipping_the_ones_imported() {
        let base = "https://w.example.org";
        let q = Query {
            limit: 3,
            ..Query::default()
        };
        let f = MockFetcher::new()
            .with_body(&entries_url(base, &q, 1), page_json(&[10, 9, 8], 1, 2))
            .with_body(&entries_url(base, &q, 2), page_json(&[7, 6], 2, 2));
        let got = fetch_entries(&f, base, "TOK", &q, &|id| id == 9).unwrap();
        assert_eq!(got.iter().map(|e| e.id).collect::<Vec<_>>(), vec![10, 8, 7]);
        // the token travels in the header, never in the address
        for r in f.requests() {
            assert_eq!(r.header_value("authorization"), Some("Bearer TOK"));
            assert!(!r.url.contains("TOK"));
        }
        // the second page is only asked for when needed
        let q1 = Query {
            limit: 1,
            ..Query::default()
        };
        let f = MockFetcher::new().with_body(&entries_url(base, &q1, 1), page_json(&[10, 9], 1, 5));
        assert_eq!(
            fetch_entries(&f, base, "T", &q1, &|_| false).unwrap().len(),
            1
        );
        assert_eq!(f.requests().len(), 1);
        // everything already imported: nothing, and the walk stops at the last page
        let q2 = Query::default();
        let f = MockFetcher::new().with_body(&entries_url(base, &q2, 1), page_json(&[1, 2], 1, 1));
        assert!(
            fetch_entries(&f, base, "T", &q2, &|_| true)
                .unwrap()
                .is_empty()
        );
        assert_eq!(f.requests().len(), 1);
    }

    #[test]
    fn a_server_that_never_ends_is_cut_off() {
        let base = "https://w.example.org";
        let q = Query {
            limit: 200,
            ..Query::default()
        };
        // every page lists the same entry and claims there are a million pages
        let f = MockFetcher::new().on_prefix(
            &format!("{base}/api/entries.json"),
            MockReply::Body(page_json(&[1], 1, 1_000_000).into_bytes()),
        );
        let got = fetch_entries(&f, base, "T", &q, &|_| false).unwrap();
        assert_eq!(got.len(), 1, "the same entry is not taken twice");
        assert_eq!(f.requests().len(), MAX_PAGES);
    }

    #[test]
    fn the_query_becomes_the_address() {
        let q = Query {
            unread_only: true,
            starred_only: true,
            tag: Some("to read".into()),
            limit: 5,
        };
        let u = entries_url("https://w.example.org", &q, 2);
        assert!(u.contains("page=2") && u.contains("archive=0") && u.contains("starred=1"));
        assert!(u.contains("tags=to%20read"), "{u}");
        let all = entries_url(
            "https://w.example.org",
            &Query {
                unread_only: false,
                ..Query::default()
            },
            1,
        );
        assert!(!all.contains("archive=") && !all.contains("starred="));
    }

    #[test]
    fn errors_say_what_to_do() {
        let base = "https://w.example.org";
        let q = Query::default();
        let f = MockFetcher::new().on(&entries_url(base, &q, 1), MockReply::Status(401));
        let e = fetch_entries(&f, base, "old", &q, &|_| false).unwrap_err();
        assert!(format!("{e:#}").contains("sign in again"), "{e:#}");
        let g = MockFetcher::new();
        let e = fetch_entries(&g, base, "t", &q, &|_| false).unwrap_err();
        assert!(format!("{e:#}").contains("no Wallabag API"), "{e:#}");
    }

    #[test]
    fn article_text_goes_through_the_whitelist() {
        let (e, _) = parse_page(page_json(&[5], 1, 1).as_bytes()).unwrap();
        let a = entry_article(&e[0]);
        assert!(a.content.contains("Text of post 5"), "{}", a.content);
        assert!(
            !a.content.contains("script") && !a.content.contains("alert"),
            "{}",
            a.content
        );
        assert_eq!(a.images, vec!["https://blog.example/p5.png"]);
        assert_eq!(a.site, "blog.example");
        assert_eq!(a.author, "Pat Writer, Sam");
        assert_eq!(
            a.lead_image.as_deref(),
            Some("https://blog.example/lead5.jpg")
        );
        assert!(a.text_len > 5);
    }
}
