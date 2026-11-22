//! Online metadata sources (L6/S6): Open Library, Google Books and Wikidata, and a [`SourceSet`]
//! that queries them in priority order, ranks the candidates and merges the best ones.
//!
//! Each source is `build request` + `parse response` + a [`Fetcher`]; parsers are pure and tested
//! with recorded-looking JSON, only [`crate::fetch::UreqFetcher`] touches the network.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{Context, Result, anyhow, bail};
use bookx_core::contracts::{MetadataQuery, MetadataSource};
use bookx_core::opf::{Identifier, Metadata};
use regex::Regex;
use serde_json::Value;

use crate::authors::same_author;
use crate::fetch::{Fetcher, Request, url_encode};
use crate::isbn;
use crate::text::{jaro_winkler, normalize, strip_html, title_similarity};

// ---------------------------------------------------------------------------------------------
// shared parsing helpers
// ---------------------------------------------------------------------------------------------

/// One result of one source.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub meta: Metadata,
    pub cover_url: Option<String>,
    /// Id of the source that produced it (`"openlibrary"`, `"google"`, `"wikidata"`).
    pub source: String,
}

fn text(v: &Value) -> String {
    v.as_str().map(|s| s.trim().to_string()).unwrap_or_default()
}

fn str_list(v: &Value) -> Vec<String> {
    match v {
        Value::Array(a) => a.iter().map(text).filter(|s| !s.is_empty()).collect(),
        Value::String(s) if !s.trim().is_empty() => vec![s.trim().to_string()],
        _ => Vec::new(),
    }
}

fn parse_json(body: &[u8], who: &str) -> Result<Value> {
    serde_json::from_slice(body).with_context(|| format!("{who}: response is not valid JSON"))
}

fn push_isbn(ids: &mut Vec<Identifier>, value: &str) {
    let n = isbn::normalize(value);
    if isbn::is_valid(&n) && !ids.iter().any(|i| i.scheme == "isbn" && i.value == n) {
        ids.push(Identifier {
            scheme: "isbn".into(),
            value: n,
            xml_id: None,
        });
    }
}

fn push_id(ids: &mut Vec<Identifier>, scheme: &str, value: &str) {
    let v = value.trim();
    if !v.is_empty() && !ids.iter().any(|i| i.scheme == scheme && i.value == v) {
        ids.push(Identifier {
            scheme: scheme.into(),
            value: v.to_string(),
            xml_id: None,
        });
    }
}

/// Turns the many date spellings found in catalogues into `YYYY`, `YYYY-MM` or `YYYY-MM-DD`.
/// Unusable input gives an empty string.
pub fn normalize_date(s: &str) -> String {
    static RES: OnceLock<[Regex; 4]> = OnceLock::new();
    let [iso, year_re, month_re, day_re] = RES.get_or_init(|| {
        [
            Regex::new(r"^(\d{4})-(\d{1,2})(?:-(\d{1,2}))?(?:$|[T ])").expect("static regex"),
            Regex::new(r"\b(1[0-9]{3}|20[0-9]{2})\b").expect("static regex"),
            Regex::new(r"(?i)\b(jan|feb|mar|apr|may|jun|jul|aug|sep|oct|nov|dec)[a-z]*\b")
                .expect("static regex"),
            Regex::new(r"\b(\d{1,2})(?:st|nd|rd|th)?\b").expect("static regex"),
        ]
    });
    let s = s.trim().trim_start_matches('+');
    if s.is_empty() {
        return String::new();
    }
    if let Some(c) = iso.captures(s) {
        let y: u32 = c[1].parse().unwrap_or(0);
        if y == 0 {
            return String::new();
        }
        let m: u32 = c[2].parse().unwrap_or(0);
        if !(1..=12).contains(&m) {
            return format!("{y:04}");
        }
        return match c.get(3).and_then(|d| d.as_str().parse::<u32>().ok()) {
            Some(d) if (1..=31).contains(&d) => format!("{y:04}-{m:02}-{d:02}"),
            _ => format!("{y:04}-{m:02}"),
        };
    }
    let Some(ym) = year_re.find(s) else {
        return String::new();
    };
    let year = ym.as_str();
    let rest = format!("{} {}", &s[..ym.start()], &s[ym.end()..]);
    let Some(mc) = month_re.captures(&rest) else {
        return year.to_string();
    };
    let month = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| mc[1].to_lowercase() == *m)
    .map(|i| i + 1)
    .unwrap_or(1);
    let day = day_re
        .captures(&rest)
        .and_then(|c| c[1].parse::<u32>().ok())
        .filter(|d| (1..=31).contains(d));
    match day {
        Some(d) => format!("{year}-{month:02}-{d:02}"),
        None => format!("{year}-{month:02}"),
    }
}

/// Maps ISO 639-2 (`eng`) or 639-1 codes to the two-letter form where known; others pass through.
pub fn normalize_language(code: &str) -> String {
    let c = code.trim().to_lowercase();
    let c = c.rsplit('/').next().unwrap_or("").to_string();
    let two = match c.as_str() {
        "eng" => "en",
        "fre" | "fra" => "fr",
        "ger" | "deu" => "de",
        "spa" => "es",
        "ita" => "it",
        "por" => "pt",
        "rus" => "ru",
        "jpn" => "ja",
        "chi" | "zho" => "zh",
        "lat" => "la",
        "dut" | "nld" => "nl",
        "swe" => "sv",
        "pol" => "pl",
        "dan" => "da",
        "nor" => "no",
        "fin" => "fi",
        "gre" | "ell" => "el",
        "tur" => "tr",
        "kor" => "ko",
        "ara" => "ar",
        "heb" => "he",
        "hin" => "hi",
        "cze" | "ces" => "cs",
        "hun" => "hu",
        other => other,
    };
    two.to_string()
}

/// Splits `Discworld ; 30`, `Discworld, #30`, `Discworld (30)`, `Dune, Book 1` into name and index.
pub fn split_series(s: &str) -> (String, Option<f64>) {
    let s = s.trim();
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)^(.*?)\s*(?:[,;]\s*(?:#|no\.?\s*|book\s+|vol(?:ume)?\.?\s*)?|\s#|\s\()(\d+(?:\.\d+)?)\)?$",
        )
        .expect("static regex")
    });
    match re.captures(s) {
        Some(c) if !c[1].trim().is_empty() => (c[1].trim().to_string(), c[2].parse().ok()),
        _ => (s.to_string(), None),
    }
}

fn cover_key(m: &Metadata) -> String {
    format!(
        "{}|{}",
        normalize(&m.title),
        m.authors.first().map(|a| normalize(a)).unwrap_or_default()
    )
}

fn query_is_empty(q: &MetadataQuery) -> bool {
    q.title.trim().is_empty()
        && q.authors.iter().all(|a| a.trim().is_empty())
        && q.isbn.as_deref().is_none_or(|i| i.trim().is_empty())
}

fn query_isbn(q: &MetadataQuery) -> Option<String> {
    q.isbn
        .as_deref()
        .map(isbn::normalize)
        .filter(|i| isbn::is_valid(i))
}

/// A source that yields rich [`Candidate`]s (metadata plus cover URL).
pub trait CandidateSource: Send + Sync {
    fn id(&self) -> &'static str;
    fn candidates(&self, query: &MetadataQuery) -> Result<Vec<Candidate>>;
}

/// Covers are cached by title/author because [`MetadataSource::cover`] only receives metadata.
#[derive(Default)]
struct CoverCache(Mutex<HashMap<String, String>>);

impl CoverCache {
    fn remember(&self, cands: &[Candidate]) {
        let mut m = self.0.lock().unwrap_or_else(|p| p.into_inner());
        for c in cands {
            if let Some(u) = &c.cover_url {
                m.insert(cover_key(&c.meta), u.clone());
            }
        }
    }

    fn get(&self, meta: &Metadata) -> Option<String> {
        self.0
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get(&cover_key(meta))
            .cloned()
    }
}

fn download_cover(fetcher: &dyn Fetcher, url: &str) -> Result<Option<Vec<u8>>> {
    let bytes = fetcher.get(url, &[])?;
    if is_image(&bytes) {
        Ok(Some(bytes))
    } else {
        // Open Library answers 200 with a 1x1 gif for unknown covers; treat as "no cover"
        Ok(None)
    }
}

fn is_image(b: &[u8]) -> bool {
    b.len() > 1000 && (b.starts_with(&[0xFF, 0xD8, 0xFF]) || b.starts_with(b"\x89PNG\r\n\x1a\n"))
}

// ---------------------------------------------------------------------------------------------
// Open Library
// ---------------------------------------------------------------------------------------------

pub struct OpenLibrary {
    fetcher: Arc<dyn Fetcher>,
    covers: CoverCache,
}

impl OpenLibrary {
    pub const ID: &'static str = "openlibrary";

    pub fn new(fetcher: Arc<dyn Fetcher>) -> Self {
        OpenLibrary {
            fetcher,
            covers: CoverCache::default(),
        }
    }

    pub fn cover_url(cover_id: u64) -> String {
        format!("https://covers.openlibrary.org/b/id/{cover_id}-L.jpg")
    }

    pub fn cover_url_for_isbn(isbn: &str) -> String {
        format!("https://covers.openlibrary.org/b/isbn/{isbn}-L.jpg?default=false")
    }

    /// `/search.json?title=&author=…`
    pub fn search_request(q: &MetadataQuery) -> Request {
        let mut url = String::from("https://openlibrary.org/search.json?");
        let mut parts: Vec<String> = Vec::new();
        if !q.title.trim().is_empty() {
            parts.push(format!("title={}", url_encode(q.title.trim())));
        }
        if let Some(a) = q.authors.iter().find(|a| !a.trim().is_empty()) {
            parts.push(format!("author={}", url_encode(a.trim())));
        }
        parts.push("limit=10".into());
        parts.push(
            "fields=key,title,subtitle,author_name,publisher,first_publish_year,publish_date,isbn,subject,language,cover_i,series"
                .into(),
        );
        url.push_str(&parts.join("&"));
        Request::new(url).header("Accept", "application/json")
    }

    /// `/isbn/<isbn>.json`
    pub fn isbn_request(isbn: &str) -> Request {
        Request::new(format!(
            "https://openlibrary.org/isbn/{}.json",
            isbn::normalize(isbn)
        ))
        .header("Accept", "application/json")
    }

    /// `/authors/<OL…A>.json`
    pub fn author_request(key: &str) -> Request {
        Request::new(format!(
            "https://openlibrary.org{}.json",
            if key.starts_with('/') {
                key.to_string()
            } else {
                format!("/authors/{key}")
            }
        ))
        .header("Accept", "application/json")
    }

    /// Parses a `search.json` response.
    pub fn parse_search(body: &[u8]) -> Result<Vec<Candidate>> {
        let v = parse_json(body, "Open Library")?;
        let docs = v
            .get("docs")
            .and_then(Value::as_array)
            .ok_or_else(|| anyhow!("Open Library: response has no `docs`"))?;
        let mut out = Vec::new();
        for d in docs {
            let mut title = text(&d["title"]);
            if title.is_empty() {
                continue;
            }
            let sub = text(&d["subtitle"]);
            if !sub.is_empty() {
                title = format!("{title}: {sub}");
            }
            let mut m = Metadata {
                title,
                authors: str_list(&d["author_name"]),
                publisher: str_list(&d["publisher"])
                    .into_iter()
                    .next()
                    .unwrap_or_default(),
                tags: str_list(&d["subject"]).into_iter().take(10).collect(),
                language: str_list(&d["language"])
                    .first()
                    .map(|l| normalize_language(l))
                    .unwrap_or_default(),
                ..Default::default()
            };
            m.published = match &d["first_publish_year"] {
                Value::Number(n) => normalize_date(&n.to_string()),
                _ => str_list(&d["publish_date"])
                    .first()
                    .map(|s| normalize_date(s))
                    .unwrap_or_default(),
            };
            for i in str_list(&d["isbn"]).iter().take(40) {
                push_isbn(&mut m.identifiers, i);
                if m.identifiers.len() >= 2 {
                    break;
                }
            }
            if let Some(key) = d["key"].as_str() {
                push_id(
                    &mut m.identifiers,
                    "olid",
                    key.rsplit('/').next().unwrap_or(""),
                );
            }
            if let Some(s) = str_list(&d["series"]).first() {
                let (name, idx) = split_series(s);
                m.series = name;
                m.series_index = idx.unwrap_or(0.0);
            }
            let cover_url = d["cover_i"].as_u64().map(Self::cover_url);
            out.push(Candidate {
                meta: m,
                cover_url,
                source: Self::ID.into(),
            });
        }
        Ok(out)
    }

    /// Parses an `/isbn/<isbn>.json` edition record. Authors are keys only; see
    /// [`OpenLibrary::author_keys`] and [`OpenLibrary::parse_author`].
    pub fn parse_isbn(body: &[u8]) -> Result<Candidate> {
        let v = parse_json(body, "Open Library")?;
        if !v.is_object() {
            bail!("Open Library: edition record is not an object");
        }
        let mut title = text(&v["title"]);
        if title.is_empty() {
            bail!("Open Library: edition record has no title");
        }
        let sub = text(&v["subtitle"]);
        if !sub.is_empty() {
            title = format!("{title}: {sub}");
        }
        let description = match &v["description"] {
            Value::String(s) => s.trim().to_string(),
            Value::Object(o) => o.get("value").map(text).unwrap_or_default(),
            _ => String::new(),
        };
        let mut m = Metadata {
            title,
            publisher: str_list(&v["publishers"])
                .into_iter()
                .next()
                .unwrap_or_default(),
            published: normalize_date(&text(&v["publish_date"])),
            description,
            tags: str_list(&v["subjects"]).into_iter().take(10).collect(),
            language: v["languages"]
                .as_array()
                .and_then(|a| a.first())
                .and_then(|l| l["key"].as_str())
                .map(normalize_language)
                .unwrap_or_default(),
            ..Default::default()
        };
        for k in ["isbn_13", "isbn_10"] {
            for i in str_list(&v[k]) {
                push_isbn(&mut m.identifiers, &i);
            }
        }
        if let Some(key) = v["key"].as_str() {
            push_id(
                &mut m.identifiers,
                "olid",
                key.rsplit('/').next().unwrap_or(""),
            );
        }
        if let Some(s) = str_list(&v["series"]).first() {
            let (name, idx) = split_series(s);
            m.series = name;
            m.series_index = idx.unwrap_or(0.0);
        }
        let cover_url = v["covers"]
            .as_array()
            .and_then(|a| a.iter().filter_map(Value::as_u64).find(|id| *id > 0))
            .map(Self::cover_url);
        Ok(Candidate {
            meta: m,
            cover_url,
            source: Self::ID.into(),
        })
    }

    /// Author keys (`/authors/OL26320A`) of an edition record.
    pub fn author_keys(body: &[u8]) -> Vec<String> {
        let Ok(v) = serde_json::from_slice::<Value>(body) else {
            return Vec::new();
        };
        v["authors"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|x| x["key"].as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Name from an `/authors/<key>.json` record.
    pub fn parse_author(body: &[u8]) -> Result<String> {
        let v = parse_json(body, "Open Library")?;
        let n = text(&v["name"]);
        if n.is_empty() {
            bail!("Open Library: author record has no name");
        }
        Ok(n)
    }
}

impl CandidateSource for OpenLibrary {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn candidates(&self, q: &MetadataQuery) -> Result<Vec<Candidate>> {
        if query_is_empty(q) {
            bail!("Open Library: empty query");
        }
        let mut out = Vec::new();
        let mut isbn_error = None;
        if let Some(i) = query_isbn(q) {
            match self.fetcher.fetch(&Self::isbn_request(&i)) {
                Ok(body) => {
                    let mut c = Self::parse_isbn(&body)?;
                    for key in Self::author_keys(&body).into_iter().take(3) {
                        if let Ok(b) = self.fetcher.fetch(&Self::author_request(&key)) {
                            if let Ok(n) = Self::parse_author(&b) {
                                c.meta.authors.push(n);
                            }
                        }
                    }
                    push_isbn(&mut c.meta.identifiers, &i);
                    if c.cover_url.is_none() {
                        c.cover_url = Some(Self::cover_url_for_isbn(&i));
                    }
                    out.push(c);
                }
                Err(e) => isbn_error = Some(e),
            }
        }
        if !q.title.trim().is_empty() {
            let body = self.fetcher.fetch(&Self::search_request(q))?;
            out.extend(Self::parse_search(&body)?);
        } else if out.is_empty() {
            if let Some(e) = isbn_error {
                return Err(e.context("Open Library ISBN lookup failed"));
            }
        }
        self.covers.remember(&out);
        Ok(out)
    }
}

impl MetadataSource for OpenLibrary {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn lookup(&self, query: &MetadataQuery) -> Result<Vec<Metadata>> {
        Ok(self
            .candidates(query)?
            .into_iter()
            .map(|c| c.meta)
            .collect())
    }

    fn cover(&self, meta: &Metadata) -> Result<Option<Vec<u8>>> {
        let url = self.covers.get(meta).or_else(|| {
            meta.identifiers
                .iter()
                .find(|i| i.scheme == "isbn")
                .map(|i| Self::cover_url_for_isbn(&i.value))
        });
        match url {
            Some(u) => download_cover(&*self.fetcher, &u),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Google Books
// ---------------------------------------------------------------------------------------------

pub struct GoogleBooks {
    fetcher: Arc<dyn Fetcher>,
    api_key: Option<String>,
    covers: CoverCache,
}

impl GoogleBooks {
    pub const ID: &'static str = "google";

    pub fn new(fetcher: Arc<dyn Fetcher>, api_key: Option<String>) -> Self {
        GoogleBooks {
            fetcher,
            api_key,
            covers: CoverCache::default(),
        }
    }

    /// `/books/v1/volumes?q=isbn:…` or `q=intitle:"…"+inauthor:"…"`.
    pub fn search_request(q: &MetadataQuery, api_key: Option<&str>) -> Request {
        let terms = if let Some(i) = query_isbn(q) {
            format!("isbn:{i}")
        } else {
            let mut t: Vec<String> = Vec::new();
            if !q.title.trim().is_empty() {
                t.push(format!("intitle:{}", quote_term(q.title.trim())));
            }
            if let Some(a) = q.authors.iter().find(|a| !a.trim().is_empty()) {
                t.push(format!("inauthor:{}", quote_term(a.trim())));
            }
            t.join("+")
        };
        let encoded: String = terms
            .split('+')
            .map(url_encode)
            .collect::<Vec<_>>()
            .join("+");
        let mut url =
            format!("https://www.googleapis.com/books/v1/volumes?q={encoded}&maxResults=10");
        if let Some(k) = api_key.filter(|k| !k.is_empty()) {
            url.push_str(&format!("&key={}", url_encode(k)));
        }
        Request::new(url).header("Accept", "application/json")
    }

    pub fn parse_volumes(body: &[u8]) -> Result<Vec<Candidate>> {
        let v = parse_json(body, "Google Books")?;
        if let Some(e) = v.get("error") {
            bail!(
                "Google Books: {}",
                e["message"].as_str().unwrap_or("API error")
            );
        }
        let Some(items) = v.get("items").and_then(Value::as_array) else {
            return Ok(Vec::new()); // `totalItems: 0` has no `items`
        };
        let mut out = Vec::new();
        for it in items {
            let info = &it["volumeInfo"];
            let mut title = text(&info["title"]);
            if title.is_empty() {
                continue;
            }
            let sub = text(&info["subtitle"]);
            if !sub.is_empty() {
                title = format!("{title}: {sub}");
            }
            let mut m = Metadata {
                title,
                authors: str_list(&info["authors"]),
                publisher: text(&info["publisher"]),
                published: normalize_date(&text(&info["publishedDate"])),
                description: strip_html(&text(&info["description"])),
                language: normalize_language(&text(&info["language"])),
                ..Default::default()
            };
            for cat in str_list(&info["categories"]) {
                for part in cat.split('/') {
                    let t = part.trim();
                    if !t.is_empty() && !m.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                        m.tags.push(t.to_string());
                    }
                }
            }
            if let Some(ids) = info["industryIdentifiers"].as_array() {
                for id in ids {
                    let kind = text(&id["type"]);
                    let val = text(&id["identifier"]);
                    if kind.starts_with("ISBN") {
                        push_isbn(&mut m.identifiers, &val);
                    }
                }
            }
            if let Some(id) = it["id"].as_str() {
                push_id(&mut m.identifiers, "google", id);
            }
            if let Some(n) = info["seriesInfo"]["bookDisplayNumber"]
                .as_str()
                .and_then(|s| s.trim().parse::<f64>().ok())
            {
                m.series_index = n;
            }
            let cover_url = [
                "extraLarge",
                "large",
                "medium",
                "thumbnail",
                "smallThumbnail",
            ]
            .iter()
            .find_map(|k| info["imageLinks"][*k].as_str())
            .map(|u| {
                u.replacen("http://", "https://", 1)
                    .replace("&edge=curl", "")
            });
            out.push(Candidate {
                meta: m,
                cover_url,
                source: Self::ID.into(),
            });
        }
        Ok(out)
    }
}

fn quote_term(s: &str) -> String {
    let clean = s.replace('"', " ");
    if clean.contains(' ') {
        format!("\"{clean}\"")
    } else {
        clean
    }
}

impl CandidateSource for GoogleBooks {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn candidates(&self, q: &MetadataQuery) -> Result<Vec<Candidate>> {
        if query_is_empty(q) {
            bail!("Google Books: empty query");
        }
        let body = self
            .fetcher
            .fetch(&Self::search_request(q, self.api_key.as_deref()))?;
        let out = Self::parse_volumes(&body)?;
        self.covers.remember(&out);
        Ok(out)
    }
}

impl MetadataSource for GoogleBooks {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn lookup(&self, query: &MetadataQuery) -> Result<Vec<Metadata>> {
        Ok(self
            .candidates(query)?
            .into_iter()
            .map(|c| c.meta)
            .collect())
    }

    fn cover(&self, meta: &Metadata) -> Result<Option<Vec<u8>>> {
        match self.covers.get(meta) {
            Some(u) => download_cover(&*self.fetcher, &u),
            None => Ok(None),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Wikidata
// ---------------------------------------------------------------------------------------------

pub struct Wikidata {
    fetcher: Arc<dyn Fetcher>,
}

fn sparql_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' | '\r' | '\t' => out.push(' '),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Wikidata {
    pub const ID: &'static str = "wikidata";

    pub fn new(fetcher: Arc<dyn Fetcher>) -> Self {
        Wikidata { fetcher }
    }

    /// The SPARQL text for a query (ISBN lookup, else entity search by title).
    pub fn sparql(q: &MetadataQuery) -> String {
        let selector = if let Some(i) = query_isbn(q) {
            format!(
                "?item wdt:P212|wdt:P957 ?anyIsbn . FILTER(REPLACE(STR(?anyIsbn), \"-\", \"\") = {})",
                sparql_string(&i)
            )
        } else {
            format!(
                "SERVICE wikibase:mwapi {{ bd:serviceParam wikibase:api \"EntitySearch\" ; wikibase:endpoint \"www.wikidata.org\" ; mwapi:search {} ; mwapi:language \"en\" . ?item wikibase:apiOutputItem mwapi:item . }} VALUES ?type {{ wd:Q571 wd:Q47461344 wd:Q7725634 wd:Q3331189 }} ?item wdt:P31/wdt:P279* ?type .",
                sparql_string(q.title.trim())
            )
        };
        format!(
            "SELECT ?item ?itemLabel ?authorLabel ?publisherLabel ?pub ?seriesLabel ?isbn13 ?isbn10 ?genreLabel WHERE {{ {selector} OPTIONAL {{ ?item wdt:P50 ?author . }} OPTIONAL {{ ?item wdt:P123 ?publisher . }} OPTIONAL {{ ?item wdt:P577 ?pub . }} OPTIONAL {{ ?item wdt:P179 ?series . }} OPTIONAL {{ ?item wdt:P212 ?isbn13 . }} OPTIONAL {{ ?item wdt:P957 ?isbn10 . }} OPTIONAL {{ ?item wdt:P136 ?genre . }} SERVICE wikibase:label {{ bd:serviceParam wikibase:language \"en\" . }} }} LIMIT 60"
        )
    }

    pub fn search_request(q: &MetadataQuery) -> Request {
        Request::new(format!(
            "https://query.wikidata.org/sparql?format=json&query={}",
            url_encode(&Self::sparql(q))
        ))
        .header("Accept", "application/sparql-results+json")
    }

    /// Parses a SPARQL JSON result; rows of one `?item` are folded into one candidate.
    pub fn parse_sparql(body: &[u8]) -> Result<Vec<Candidate>> {
        let v = parse_json(body, "Wikidata")?;
        let rows = v["results"]["bindings"]
            .as_array()
            .ok_or_else(|| anyhow!("Wikidata: response has no `results.bindings`"))?;
        let mut order: Vec<String> = Vec::new();
        let mut by_item: HashMap<String, Metadata> = HashMap::new();
        for row in rows {
            let get = |k: &str| {
                row[k]["value"]
                    .as_str()
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default()
            };
            let item = get("item");
            if item.is_empty() {
                continue;
            }
            let label = get("itemLabel");
            // unresolved labels come back as the bare Q-id
            let is_qid = label.starts_with('Q') && label[1..].chars().all(|c| c.is_ascii_digit());
            let m = by_item.entry(item.clone()).or_insert_with(|| {
                order.push(item.clone());
                Metadata::default()
            });
            if m.title.is_empty() && !label.is_empty() && !is_qid {
                m.title = label;
            }
            let author = get("authorLabel");
            if !author.is_empty() && !m.authors.contains(&author) {
                m.authors.push(author);
            }
            if m.publisher.is_empty() {
                m.publisher = get("publisherLabel");
            }
            if m.published.is_empty() {
                m.published = normalize_date(&get("pub"));
            }
            if m.series.is_empty() {
                m.series = get("seriesLabel");
            }
            let genre = get("genreLabel");
            if !genre.is_empty() && !m.tags.contains(&genre) {
                m.tags.push(genre);
            }
            push_isbn(&mut m.identifiers, &get("isbn13"));
            push_isbn(&mut m.identifiers, &get("isbn10"));
            push_id(
                &mut m.identifiers,
                "wikidata",
                item.rsplit('/').next().unwrap_or(""),
            );
        }
        Ok(order
            .into_iter()
            .filter_map(|k| by_item.remove(&k))
            .filter(|m| !m.title.is_empty())
            .map(|meta| Candidate {
                meta,
                cover_url: None,
                source: Self::ID.into(),
            })
            .collect())
    }
}

impl CandidateSource for Wikidata {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn candidates(&self, q: &MetadataQuery) -> Result<Vec<Candidate>> {
        if query_isbn(q).is_none() && q.title.trim().is_empty() {
            bail!("Wikidata: needs a title or an ISBN");
        }
        let body = self.fetcher.fetch(&Self::search_request(q))?;
        Self::parse_sparql(&body)
    }
}

impl MetadataSource for Wikidata {
    fn id(&self) -> &'static str {
        Self::ID
    }

    fn lookup(&self, query: &MetadataQuery) -> Result<Vec<Metadata>> {
        Ok(self
            .candidates(query)?
            .into_iter()
            .map(|c| c.meta)
            .collect())
    }
}

// ---------------------------------------------------------------------------------------------
// ranking and merging
// ---------------------------------------------------------------------------------------------

/// Fields that can have their own source priority.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Field {
    Title,
    Authors,
    Publisher,
    Published,
    Description,
    Tags,
    Language,
    Series,
    Cover,
}

impl Field {
    pub const ALL: [Field; 9] = [
        Field::Title,
        Field::Authors,
        Field::Publisher,
        Field::Published,
        Field::Description,
        Field::Tags,
        Field::Language,
        Field::Series,
        Field::Cover,
    ];

    /// The stable name used in settings (`"description"`).
    pub fn key(self) -> &'static str {
        field_key(self)
    }

    pub fn from_key(k: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == k)
    }
}

/// The ids of the built-in sources, in their default order.
pub const SOURCE_IDS: [&str; 3] = [OpenLibrary::ID, GoogleBooks::ID, Wikidata::ID];

fn field_key(f: Field) -> &'static str {
    match f {
        Field::Title => "title",
        Field::Authors => "authors",
        Field::Publisher => "publisher",
        Field::Published => "published",
        Field::Description => "description",
        Field::Language => "language",
        Field::Series => "series",
        Field::Tags => "tags",
        Field::Cover => "cover",
    }
}

/// Per-field source order for merging; fields without an entry use the default order
/// (the order sources were added to the [`SourceSet`]).
#[derive(Debug, Clone, Default)]
pub struct FieldPriority {
    map: HashMap<Field, Vec<String>>,
}

impl FieldPriority {
    /// The user's choices (`field key` → source ids, first is preferred) on top of `self`. Unknown
    /// fields and unknown source ids are ignored; a field with no valid source keeps `self`'s order.
    pub fn overlay(mut self, user: &std::collections::BTreeMap<String, Vec<String>>) -> Self {
        for (k, ids) in user {
            let Some(f) = Field::from_key(k) else {
                continue;
            };
            let ids: Vec<String> = ids
                .iter()
                .filter(|i| SOURCE_IDS.contains(&i.as_str()))
                .cloned()
                .collect();
            if !ids.is_empty() {
                self.map.insert(f, ids);
            }
        }
        self
    }

    /// The first source of the order set for `field`, if any.
    pub fn preferred(&self, field: Field) -> Option<&str> {
        self.map
            .get(&field)
            .and_then(|l| l.first())
            .map(String::as_str)
    }

    pub fn prefer(mut self, field: Field, sources: &[&str]) -> Self {
        self.map
            .insert(field, sources.iter().map(|s| s.to_string()).collect());
        self
    }

    /// Rank of `source` for `field` (lower = preferred); unlisted sources come after listed ones,
    /// ordered by `default_rank`.
    fn rank(&self, field: Field, source: &str, default_rank: usize) -> usize {
        match self
            .map
            .get(&field)
            .and_then(|l| l.iter().position(|s| s == source))
        {
            Some(p) => p,
            None => 1000 + default_rank,
        }
    }
}

/// Similarity between the query's authors and a candidate's, in 0..=1.
pub fn author_score(wanted: &[String], found: &[String]) -> f64 {
    let wanted: Vec<&String> = wanted.iter().filter(|a| !a.trim().is_empty()).collect();
    if wanted.is_empty() {
        return 1.0;
    }
    if found.is_empty() {
        return 0.5;
    }
    let total: f64 = wanted
        .iter()
        .map(|w| {
            found
                .iter()
                .map(|f| {
                    if same_author(w, f) {
                        1.0
                    } else {
                        let jw = jaro_winkler(&normalize(w), &normalize(f));
                        if jw >= 0.88 { jw * 0.9 } else { jw * 0.4 }
                    }
                })
                .fold(0.0, f64::max)
        })
        .sum();
    total / wanted.len() as f64
}

/// Match quality of a candidate for a query: 1.0 on ISBN equality, else 0.7 × title + 0.3 × authors
/// (title only when the query has no authors).
pub fn score_candidate(q: &MetadataQuery, c: &Candidate) -> f64 {
    if let Some(qi) = query_isbn(q) {
        if c.meta
            .identifiers
            .iter()
            .any(|i| i.scheme == "isbn" && isbn::same_isbn(&i.value, &qi))
        {
            return 1.0;
        }
    }
    let title = if q.title.trim().is_empty() {
        0.5
    } else {
        title_similarity(&q.title, &c.meta.title)
    };
    if q.authors.iter().all(|a| a.trim().is_empty()) {
        title
    } else {
        0.7 * title + 0.3 * author_score(&q.authors, &c.meta.authors)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Ranked {
    pub candidate: Candidate,
    pub score: f64,
}

/// The merged answer.
#[derive(Debug, Clone, PartialEq)]
pub struct Best {
    pub meta: Metadata,
    pub cover_url: Option<String>,
    /// 0..=1; the best candidate's score plus a small bonus when several sources agree.
    pub confidence: f64,
    /// Sources that contributed to the merge, best first.
    pub sources: Vec<String>,
    /// Which source gave each field: `title`, `authors`, `series`, `publisher`, `published`,
    /// `language`, `description`, `tags` and `identifiers` (several sources joined by `, `),
    /// and `cover`.
    pub field_sources: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceError {
    pub source: String,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LookupResult {
    pub best: Option<Best>,
    /// All candidates, best first.
    pub ranked: Vec<Ranked>,
    /// Sources that failed (the others still answered).
    pub errors: Vec<SourceError>,
}

/// Queries several sources in order and combines their answers.
pub struct SourceSet {
    sources: Vec<Box<dyn CandidateSource>>,
    priority: FieldPriority,
    fetcher: Arc<dyn Fetcher>,
    /// Below this score no `best` is returned (candidates stay in `ranked`).
    pub min_confidence: f64,
    /// Candidates within this distance of the top score are merged into it.
    pub merge_window: f64,
}

impl SourceSet {
    pub fn new(fetcher: Arc<dyn Fetcher>) -> Self {
        SourceSet {
            sources: Vec::new(),
            priority: FieldPriority::default(),
            fetcher,
            min_confidence: 0.5,
            merge_window: 0.15,
        }
    }

    /// Open Library, Google Books and Wikidata over one fetcher, in that order, with Google
    /// preferred for descriptions and Open Library for tags.
    /// The built-in field order of [`standard`](Self::standard).
    pub fn standard_priority() -> FieldPriority {
        FieldPriority::default()
            .prefer(Field::Description, &[GoogleBooks::ID, OpenLibrary::ID])
            .prefer(Field::Series, &[Wikidata::ID, OpenLibrary::ID])
    }

    pub fn standard(fetcher: Arc<dyn Fetcher>) -> Self {
        let mut s = SourceSet::new(fetcher.clone());
        s.add(Box::new(OpenLibrary::new(fetcher.clone())));
        s.add(Box::new(GoogleBooks::new(fetcher.clone(), None)));
        s.add(Box::new(Wikidata::new(fetcher)));
        s.priority = Self::standard_priority();
        s
    }

    pub fn add(&mut self, source: Box<dyn CandidateSource>) -> &mut Self {
        self.sources.push(source);
        self
    }

    pub fn with_priority(mut self, p: FieldPriority) -> Self {
        self.priority = p;
        self
    }

    pub fn len(&self) -> usize {
        self.sources.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sources.is_empty()
    }

    /// Queries every source. Errors from single sources are reported in the result; an `Err` is
    /// returned only when every source failed (or there are none).
    pub fn search(&self, q: &MetadataQuery) -> Result<LookupResult> {
        if self.sources.is_empty() {
            bail!("no metadata sources configured");
        }
        let mut res = LookupResult::default();
        let mut all: Vec<Candidate> = Vec::new();
        for s in &self.sources {
            match s.candidates(q) {
                Ok(c) => all.extend(c),
                Err(e) => res.errors.push(SourceError {
                    source: s.id().to_string(),
                    message: format!("{e:#}"),
                }),
            }
        }
        if all.is_empty() && res.errors.len() == self.sources.len() {
            let msgs: Vec<String> = res
                .errors
                .iter()
                .map(|e| format!("{}: {}", e.source, e.message))
                .collect();
            bail!("all metadata sources failed: {}", msgs.join("; "));
        }
        let mut ranked: Vec<Ranked> = all
            .into_iter()
            .map(|c| Ranked {
                score: score_candidate(q, &c),
                candidate: c,
            })
            .collect();
        // stable: ties keep source order
        ranked.sort_by(|a, b| b.score.total_cmp(&a.score));
        res.best = self.merge_best(&ranked);
        res.ranked = ranked;
        Ok(res)
    }

    fn merge_best(&self, ranked: &[Ranked]) -> Option<Best> {
        let top = ranked.first()?;
        if top.score < self.min_confidence {
            return None;
        }
        let group: Vec<&Ranked> = ranked
            .iter()
            .filter(|r| r.score >= top.score - self.merge_window)
            .collect();
        let rank_of = |src: &str| {
            self.sources
                .iter()
                .position(|s| s.id() == src)
                .unwrap_or(usize::MAX / 2)
        };
        let pick = |field: Field| -> Vec<&Ranked> {
            let mut g = group.clone();
            g.sort_by_key(|r| {
                self.priority
                    .rank(field, &r.candidate.source, rank_of(&r.candidate.source))
            });
            g
        };
        let mut meta = Metadata::default();
        let mut field_sources: std::collections::BTreeMap<String, String> = Default::default();
        macro_rules! first_string {
            ($field:expr_2021, $get:expr_2021, $set:expr_2021) => {
                if let Some((v, src)) = pick($field)
                    .iter()
                    .map(|r| ($get(&r.candidate.meta), r.candidate.source.clone()))
                    .find(|(s, _): &(String, String)| !s.trim().is_empty())
                {
                    $set(&mut meta, v);
                    field_sources.insert(field_key($field).to_string(), src);
                }
            };
        }
        first_string!(
            Field::Title,
            |m: &Metadata| m.title.clone(),
            |m: &mut Metadata, v| m.title = v
        );
        first_string!(
            Field::Publisher,
            |m: &Metadata| m.publisher.clone(),
            |m: &mut Metadata, v| m.publisher = v
        );
        first_string!(
            Field::Published,
            |m: &Metadata| m.published.clone(),
            |m: &mut Metadata, v| m.published = v
        );
        first_string!(
            Field::Description,
            |m: &Metadata| m.description.clone(),
            |m: &mut Metadata, v| m.description = v
        );
        first_string!(
            Field::Language,
            |m: &Metadata| m.language.clone(),
            |m: &mut Metadata, v| m.language = v
        );
        if let Some(r) = pick(Field::Authors)
            .into_iter()
            .find(|r| !r.candidate.meta.authors.is_empty())
        {
            meta.authors = r.candidate.meta.authors.clone();
            field_sources.insert("authors".into(), r.candidate.source.clone());
        }
        if let Some(r) = pick(Field::Series)
            .into_iter()
            .find(|r| !r.candidate.meta.series.trim().is_empty())
        {
            meta.series = r.candidate.meta.series.clone();
            meta.series_index = r.candidate.meta.series_index;
            field_sources.insert("series".into(), r.candidate.source.clone());
        }
        let mut tag_sources: Vec<String> = Vec::new();
        for r in pick(Field::Tags) {
            for t in &r.candidate.meta.tags {
                if !meta.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
                    meta.tags.push(t.clone());
                    if !tag_sources.contains(&r.candidate.source) {
                        tag_sources.push(r.candidate.source.clone());
                    }
                }
            }
        }
        if !tag_sources.is_empty() {
            field_sources.insert("tags".into(), tag_sources.join(", "));
        }
        meta.tags.truncate(15);
        let mut id_sources: Vec<String> = Vec::new();
        for r in &group {
            for id in &r.candidate.meta.identifiers {
                if !meta
                    .identifiers
                    .iter()
                    .any(|x| x.scheme == id.scheme && x.value == id.value)
                {
                    meta.identifiers.push(id.clone());
                    if !id_sources.contains(&r.candidate.source) {
                        id_sources.push(r.candidate.source.clone());
                    }
                }
            }
        }
        if !id_sources.is_empty() {
            field_sources.insert("identifiers".into(), id_sources.join(", "));
        }
        let cover = pick(Field::Cover).iter().find_map(|r| {
            r.candidate
                .cover_url
                .clone()
                .map(|u| (u, r.candidate.source.clone()))
        });
        let cover_url = cover.as_ref().map(|c| c.0.clone());
        if let Some((_, src)) = cover {
            field_sources.insert("cover".into(), src);
        }
        let mut sources: Vec<String> = Vec::new();
        for r in &group {
            if !sources.contains(&r.candidate.source) {
                sources.push(r.candidate.source.clone());
            }
        }
        let bonus = 0.03 * (sources.len().saturating_sub(1)) as f64;
        Some(Best {
            meta,
            cover_url,
            confidence: (top.score + bonus).min(1.0),
            sources,
            field_sources,
        })
    }

    /// Downloads the cover of a merged result. `Ok(None)` when there is no cover URL or the
    /// response is not a JPEG/PNG.
    pub fn fetch_cover(&self, best: &Best) -> Result<Option<Vec<u8>>> {
        match &best.cover_url {
            Some(u) => download_cover(&*self.fetcher, u),
            None => Ok(None),
        }
    }
}

impl MetadataSource for SourceSet {
    fn id(&self) -> &'static str {
        "sources"
    }

    /// The merged best result first, then the remaining candidates by score.
    fn lookup(&self, query: &MetadataQuery) -> Result<Vec<Metadata>> {
        let r = self.search(query)?;
        let mut out = Vec::new();
        if let Some(b) = &r.best {
            out.push(b.meta.clone());
        }
        out.extend(r.ranked.into_iter().map(|c| c.candidate.meta));
        Ok(out)
    }

    fn cover(&self, meta: &Metadata) -> Result<Option<Vec<u8>>> {
        let q = MetadataQuery {
            title: meta.title.clone(),
            authors: meta.authors.clone(),
            isbn: meta.identifier("isbn").map(String::from),
        };
        let r = self.search(&q)?;
        match r.best {
            Some(b) => self.fetch_cover(&b),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::{FetchError, MockFetcher, MockReply};

    fn q(title: &str, authors: &[&str]) -> MetadataQuery {
        MetadataQuery {
            title: title.into(),
            authors: authors.iter().map(|s| s.to_string()).collect(),
            isbn: None,
        }
    }

    fn jpeg(n: usize) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0];
        v.resize(n, 7);
        v
    }

    const OL_SEARCH: &str = r#"{
      "numFound": 3, "start": 0, "numFoundExact": true,
      "docs": [
        {"key": "/works/OL45883W", "title": "The Hobbit", "subtitle": "There and Back Again",
         "author_name": ["J.R.R. Tolkien"], "publisher": ["Allen & Unwin", "Houghton Mifflin"],
         "first_publish_year": 1937, "isbn": ["0618260307", "9780618260300", "bad-isbn", "0261102214"],
         "subject": ["Fantasy", "Dragons", "Hobbits"], "language": ["eng", "fre"], "cover_i": 8406786,
         "series": ["Middle-earth ; 0"]},
        {"key": "/works/OL1W", "title": "Good Omens",
         "author_name": ["Terry Pratchett", "Neil Gaiman"], "publish_date": ["May 1990", "1991"]},
        {"key": "/works/OL2W", "title": "Les Misérables – 日本語版", "author_name": ["Victor Hugo"],
         "first_publish_year": 1862},
        {"key": "/works/OL3W"},
        {"title": ""}
      ]
    }"#;

    #[test]
    fn date_normalisation() {
        for (input, want) in [
            ("2003", "2003"),
            ("2003-05", "2003-05"),
            ("2003-05-01", "2003-05-01"),
            ("2003-05-01T00:00:00Z", "2003-05-01"),
            ("+2003-05-01T00:00:00Z", "2003-05-01"),
            ("2003-13-01", "2003"),
            ("2003-05-45", "2003-05"),
            ("2003-00-00", "2003"),
            ("0000-00-00", ""),
            ("May 2003", "2003-05"),
            ("May 5, 2003", "2003-05-05"),
            ("5th May 2003", "2003-05-05"),
            ("Sept. 1999", "1999-09"),
            ("c. 1850", "1850"),
            ("[1850?]", "1850"),
            ("Published in the year 1999 by someone", "1999"),
            ("sometime", ""),
            ("", ""),
            ("  ", ""),
            ("12345", ""),
            ("1899-12-31 23:59", "1899-12-31"),
        ] {
            assert_eq!(normalize_date(input), want, "input {input:?}");
        }
    }

    #[test]
    fn language_normalisation() {
        assert_eq!(normalize_language("eng"), "en");
        assert_eq!(normalize_language("/languages/fre"), "fr");
        assert_eq!(normalize_language("DEU"), "de");
        assert_eq!(normalize_language("en"), "en");
        assert_eq!(normalize_language("tlh"), "tlh");
        assert_eq!(normalize_language(""), "");
    }

    #[test]
    fn series_splitting() {
        assert_eq!(
            split_series("Discworld ; 30"),
            ("Discworld".into(), Some(30.0))
        );
        assert_eq!(
            split_series("Discworld, #30"),
            ("Discworld".into(), Some(30.0))
        );
        assert_eq!(
            split_series("Discworld #30"),
            ("Discworld".into(), Some(30.0))
        );
        assert_eq!(
            split_series("Discworld (30)"),
            ("Discworld".into(), Some(30.0))
        );
        assert_eq!(split_series("Dune, Book 1"), ("Dune".into(), Some(1.0)));
        assert_eq!(
            split_series("Cosmere; vol. 1.5"),
            ("Cosmere".into(), Some(1.5))
        );
        assert_eq!(
            split_series("Fahrenheit 451"),
            ("Fahrenheit 451".into(), None)
        );
        assert_eq!(split_series("Standalone"), ("Standalone".into(), None));
        assert_eq!(split_series(""), (String::new(), None));
        assert_eq!(
            split_series("; 3"),
            ("; 3".into(), None),
            "no name, nothing to split"
        );
    }

    #[test]
    fn open_library_search_request() {
        let r = OpenLibrary::search_request(&q("The Hobbit & more", &["J. R. R. Tolkien"]));
        assert!(r.url.starts_with("https://openlibrary.org/search.json?title=The%20Hobbit%20%26%20more&author=J.%20R.%20R.%20Tolkien&limit=10&fields="), "{}", r.url);
        assert_eq!(r.header_value("accept"), Some("application/json"));
        let only_title = OpenLibrary::search_request(&q("Dune", &[]));
        assert!(!only_title.url.contains("author="));
        assert_eq!(
            OpenLibrary::isbn_request("978-0-306-40615-7").url,
            "https://openlibrary.org/isbn/9780306406157.json"
        );
        assert_eq!(
            OpenLibrary::author_request("/authors/OL26320A").url,
            "https://openlibrary.org/authors/OL26320A.json"
        );
        assert_eq!(
            OpenLibrary::author_request("OL26320A").url,
            "https://openlibrary.org/authors/OL26320A.json"
        );
        assert_eq!(
            OpenLibrary::cover_url(12),
            "https://covers.openlibrary.org/b/id/12-L.jpg"
        );
    }

    #[test]
    fn open_library_search_parsing() {
        let c = OpenLibrary::parse_search(OL_SEARCH.as_bytes()).unwrap();
        assert_eq!(c.len(), 3, "records without a title are skipped");
        let h = &c[0];
        assert_eq!(h.source, "openlibrary");
        assert_eq!(h.meta.title, "The Hobbit: There and Back Again");
        assert_eq!(h.meta.authors, vec!["J.R.R. Tolkien"]);
        assert_eq!(h.meta.publisher, "Allen & Unwin");
        assert_eq!(h.meta.published, "1937");
        assert_eq!(h.meta.language, "en");
        assert_eq!(h.meta.tags, vec!["Fantasy", "Dragons", "Hobbits"]);
        assert_eq!(h.meta.series, "Middle-earth");
        assert_eq!(
            h.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/8406786-L.jpg")
        );
        let isbns: Vec<&str> = h
            .meta
            .identifiers
            .iter()
            .filter(|i| i.scheme == "isbn")
            .map(|i| i.value.as_str())
            .collect();
        assert_eq!(
            isbns,
            vec!["0618260307", "9780618260300"],
            "invalid ISBNs dropped, capped at two"
        );
        assert!(
            h.meta
                .identifiers
                .iter()
                .any(|i| i.scheme == "olid" && i.value == "OL45883W")
        );
    }

    #[test]
    fn open_library_search_missing_fields_and_weird_dates() {
        let c = OpenLibrary::parse_search(OL_SEARCH.as_bytes()).unwrap();
        let g = &c[1];
        assert_eq!(g.meta.authors, vec!["Terry Pratchett", "Neil Gaiman"]);
        assert_eq!(g.meta.published, "1990-05");
        assert_eq!(g.meta.publisher, "");
        assert!(g.cover_url.is_none());
        assert!(g.meta.tags.is_empty());
        let l = &c[2];
        assert_eq!(l.meta.title, "Les Misérables – 日本語版");
        assert_eq!(l.meta.published, "1862");
    }

    #[test]
    fn open_library_search_errors() {
        for (body, msg) in [
            ("not json", "not valid JSON"),
            ("[]", "no `docs`"),
            ("{}", "no `docs`"),
            ("", "not valid JSON"),
            ("{\"docs\": 5}", "no `docs`"),
        ] {
            let e = OpenLibrary::parse_search(body.as_bytes()).unwrap_err();
            assert!(format!("{e:#}").contains(msg), "{body:?}: {e:#}");
        }
        assert!(
            OpenLibrary::parse_search(b"{\"docs\": []}")
                .unwrap()
                .is_empty()
        );
    }

    const OL_EDITION: &str = r#"{
      "key": "/books/OL7353617M", "title": "Fantastic Mr Fox", "subtitle": "a tale",
      "publishers": ["Puffin"], "publish_date": "October 1, 1988",
      "isbn_10": ["0140328726"], "isbn_13": ["9780140328721"],
      "covers": [-1, 6498519], "languages": [{"key": "/languages/eng"}],
      "description": {"type": "/type/text", "value": "A fox outwits farmers."},
      "subjects": ["Foxes", "Fiction"], "series": ["Roald Dahl, #4"],
      "authors": [{"key": "/authors/OL34184A"}, {"key": "/authors/OL1A"}],
      "works": [{"key": "/works/OL45804W"}]
    }"#;

    #[test]
    fn open_library_edition_parsing() {
        let c = OpenLibrary::parse_isbn(OL_EDITION.as_bytes()).unwrap();
        let m = &c.meta;
        assert_eq!(m.title, "Fantastic Mr Fox: a tale");
        assert_eq!(m.publisher, "Puffin");
        assert_eq!(m.published, "1988-10-01");
        assert_eq!(m.description, "A fox outwits farmers.");
        assert_eq!(m.language, "en");
        assert_eq!(m.tags, vec!["Foxes", "Fiction"]);
        assert_eq!((m.series.as_str(), m.series_index), ("Roald Dahl", 4.0));
        assert_eq!(m.identifier("isbn"), Some("9780140328721"));
        assert!(m.identifiers.iter().any(|i| i.value == "0140328726"));
        assert!(
            m.identifiers
                .iter()
                .any(|i| i.scheme == "olid" && i.value == "OL7353617M")
        );
        assert_eq!(
            c.cover_url.as_deref(),
            Some("https://covers.openlibrary.org/b/id/6498519-L.jpg"),
            "negative cover ids skipped"
        );
        assert_eq!(
            OpenLibrary::author_keys(OL_EDITION.as_bytes()),
            vec!["/authors/OL34184A", "/authors/OL1A"]
        );
        assert!(OpenLibrary::author_keys(b"junk").is_empty());
    }

    #[test]
    fn open_library_edition_plain_string_description_and_minimal() {
        let c = OpenLibrary::parse_isbn(br#"{"title": "T", "description": " plain "}"#).unwrap();
        assert_eq!(c.meta.description, "plain");
        assert!(c.cover_url.is_none());
        for (body, msg) in [
            ("[]", "not an object"),
            (r#"{"x":1}"#, "no title"),
            ("{", "not valid JSON"),
        ] {
            let e = OpenLibrary::parse_isbn(body.as_bytes()).unwrap_err();
            assert!(format!("{e:#}").contains(msg), "{body}: {e:#}");
        }
    }

    #[test]
    fn open_library_author_record() {
        assert_eq!(
            OpenLibrary::parse_author(br#"{"name": " Roald Dahl "}"#).unwrap(),
            "Roald Dahl"
        );
        assert!(OpenLibrary::parse_author(b"{}").is_err());
        assert!(OpenLibrary::parse_author(b"x").is_err());
    }

    fn ol_mock() -> MockFetcher {
        MockFetcher::new()
            .with_body(
                "https://openlibrary.org/isbn/9780140328721.json",
                OL_EDITION,
            )
            .with_body(
                "https://openlibrary.org/authors/OL34184A.json",
                r#"{"name": "Roald Dahl"}"#,
            )
            .on(
                "https://openlibrary.org/authors/OL1A.json",
                MockReply::Status(404),
            )
            .on_prefix(
                "https://openlibrary.org/search.json",
                MockReply::Body(OL_SEARCH.as_bytes().to_vec()),
            )
    }

    #[test]
    fn open_library_isbn_lookup_resolves_authors() {
        let f = Arc::new(ol_mock());
        let ol = OpenLibrary::new(f.clone());
        let query = MetadataQuery {
            isbn: Some("978-0-14-032872-1".into()),
            ..Default::default()
        };
        let out = ol.candidates(&query).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(
            out[0].meta.authors,
            vec!["Roald Dahl"],
            "second author failed to resolve and is skipped"
        );
        assert_eq!(f.urls().len(), 3, "edition + two authors");
    }

    #[test]
    fn open_library_title_lookup_and_metadata_source_trait() {
        let ol = OpenLibrary::new(Arc::new(ol_mock()));
        let out = MetadataSource::lookup(&ol, &q("The Hobbit", &["Tolkien"])).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(MetadataSource::id(&ol), "openlibrary");
    }

    #[test]
    fn open_library_isbn_failure_falls_back_to_title_search() {
        let f = MockFetcher::new()
            .on_prefix("https://openlibrary.org/isbn/", MockReply::Status(404))
            .on_prefix(
                "https://openlibrary.org/search.json",
                MockReply::Body(OL_SEARCH.as_bytes().to_vec()),
            );
        let ol = OpenLibrary::new(Arc::new(f));
        let query = MetadataQuery {
            title: "The Hobbit".into(),
            isbn: Some("9780140328721".into()),
            ..Default::default()
        };
        assert_eq!(ol.candidates(&query).unwrap().len(), 3);
    }

    #[test]
    fn open_library_isbn_only_failure_is_an_error() {
        let f =
            MockFetcher::new().on_prefix("https://openlibrary.org/isbn/", MockReply::Status(404));
        let ol = OpenLibrary::new(Arc::new(f));
        let query = MetadataQuery {
            isbn: Some("9780140328721".into()),
            ..Default::default()
        };
        let e = ol.candidates(&query).unwrap_err();
        assert_eq!(FetchError::of(&e), Some(&FetchError::Http(404)));
    }

    #[test]
    fn sources_reject_empty_queries() {
        let f: Arc<dyn Fetcher> = Arc::new(MockFetcher::new());
        let empty = MetadataQuery::default();
        assert!(
            OpenLibrary::new(f.clone())
                .candidates(&empty)
                .unwrap_err()
                .to_string()
                .contains("empty query")
        );
        assert!(
            GoogleBooks::new(f.clone(), None)
                .candidates(&empty)
                .unwrap_err()
                .to_string()
                .contains("empty query")
        );
        assert!(Wikidata::new(f).candidates(&empty).is_err());
    }

    #[test]
    fn open_library_fetch_failures_are_errors_not_panics() {
        for reply in [
            MockReply::Status(500),
            MockReply::Timeout,
            MockReply::Body(b"<html>oops</html>".to_vec()),
            MockReply::Body(vec![]),
        ] {
            let f = MockFetcher::new().on_prefix("https://openlibrary.org/", reply);
            let ol = OpenLibrary::new(Arc::new(f));
            assert!(ol.candidates(&q("Dune", &[])).is_err());
        }
    }

    #[test]
    fn open_library_cover_download() {
        let f = MockFetcher::new()
            .on_prefix(
                "https://openlibrary.org/search.json",
                MockReply::Body(OL_SEARCH.as_bytes().to_vec()),
            )
            .with_body(
                "https://covers.openlibrary.org/b/id/8406786-L.jpg",
                jpeg(2000),
            )
            .with_body(
                "https://covers.openlibrary.org/b/isbn/0618260307-L.jpg?default=false",
                b"GIF89a",
            );
        let ol = OpenLibrary::new(Arc::new(f));
        let found = ol.candidates(&q("The Hobbit", &[])).unwrap();
        let cover = MetadataSource::cover(&ol, &found[0].meta).unwrap();
        assert_eq!(cover.map(|b| b.len()), Some(2000));
        // unknown book with an ISBN: falls back to the ISBN cover URL, which returns a placeholder
        let unknown = Metadata {
            title: "Other".into(),
            identifiers: vec![Identifier {
                scheme: "isbn".into(),
                value: "0618260307".into(),
                xml_id: None,
            }],
            ..Default::default()
        };
        assert_eq!(MetadataSource::cover(&ol, &unknown).unwrap(), None);
        // nothing known at all
        assert_eq!(
            MetadataSource::cover(&ol, &Metadata::default()).unwrap(),
            None
        );
    }

    const GB: &str = r#"{
      "kind": "books#volumes", "totalItems": 3,
      "items": [
        {"id": "pD6arNyKyi8C", "volumeInfo": {
          "title": "The Hobbit", "subtitle": "Or There and Back Again",
          "authors": ["J.R.R. Tolkien"], "publisher": "Houghton Mifflin Harcourt",
          "publishedDate": "2012-09-18",
          "description": "<p>In a hole in the ground there lived a <b>hobbit</b>.&nbsp;Not a nasty hole.</p>",
          "industryIdentifiers": [{"type": "ISBN_13", "identifier": "9780547928227"}, {"type": "ISBN_10", "identifier": "054792822X"}, {"type": "OTHER", "identifier": "OCLC:1"}],
          "categories": ["Fiction / Fantasy / General", "Juvenile Fiction"],
          "language": "en",
          "imageLinks": {"smallThumbnail": "http://books.google.com/small.jpg", "thumbnail": "http://books.google.com/books/content?id=pD6arNyKyi8C&printsec=frontcover&img=1&zoom=1&edge=curl&source=gbs_api"},
          "seriesInfo": {"bookDisplayNumber": "1", "volumeSeries": [{"seriesId": "x", "orderNumber": 1}]}}},
        {"id": "AAAAAAAAAAAA", "volumeInfo": {"title": "Bare", "publishedDate": "99"}},
        {"id": "noinfo"},
        {"id": "BBBBBBBBBBBB", "volumeInfo": {"title": "Ünïcödé 日本", "authors": ["A", "B", "C"], "publishedDate": "2003-05"}}
      ]
    }"#;

    #[test]
    fn google_request_for_title_author_and_isbn() {
        let r = GoogleBooks::search_request(&q("The Hobbit", &["J.R.R. Tolkien"]), Some("KEY 1"));
        assert_eq!(
            r.url,
            "https://www.googleapis.com/books/v1/volumes?q=intitle%3A%22The%20Hobbit%22+inauthor%3A%22J.R.R.%20Tolkien%22&maxResults=10&key=KEY%201"
        );
        let single = GoogleBooks::search_request(&q("Dune", &[]), None);
        assert_eq!(
            single.url,
            "https://www.googleapis.com/books/v1/volumes?q=intitle%3ADune&maxResults=10"
        );
        let by_isbn = GoogleBooks::search_request(
            &MetadataQuery {
                title: "ignored".into(),
                isbn: Some("978-0-306-40615-7".into()),
                ..Default::default()
            },
            Some(""),
        );
        assert_eq!(
            by_isbn.url,
            "https://www.googleapis.com/books/v1/volumes?q=isbn%3A9780306406157&maxResults=10"
        );
        let quoted = GoogleBooks::search_request(&q("Say \"hi\" now", &[]), None);
        assert!(
            quoted.url.contains("intitle%3A%22Say%20%20hi%20%20now%22"),
            "{}",
            quoted.url
        );
    }

    #[test]
    fn google_parsing() {
        let c = GoogleBooks::parse_volumes(GB.as_bytes()).unwrap();
        assert_eq!(c.len(), 3, "volume without info is skipped");
        let h = &c[0];
        assert_eq!(h.meta.title, "The Hobbit: Or There and Back Again");
        assert_eq!(h.meta.authors, vec!["J.R.R. Tolkien"]);
        assert_eq!(h.meta.publisher, "Houghton Mifflin Harcourt");
        assert_eq!(h.meta.published, "2012-09-18");
        assert_eq!(
            h.meta.description,
            "In a hole in the ground there lived a hobbit. Not a nasty hole."
        );
        assert_eq!(
            h.meta.tags,
            vec!["Fiction", "Fantasy", "General", "Juvenile Fiction"]
        );
        assert_eq!(h.meta.language, "en");
        assert_eq!(h.meta.series_index, 1.0);
        assert_eq!(h.meta.identifier("isbn"), Some("9780547928227"));
        assert!(h.meta.identifiers.iter().any(|i| i.value == "054792822X"));
        assert!(!h.meta.identifiers.iter().any(|i| i.value.contains("OCLC")));
        assert!(
            h.meta
                .identifiers
                .iter()
                .any(|i| i.scheme == "google" && i.value == "pD6arNyKyi8C")
        );
        assert_eq!(
            h.cover_url.as_deref(),
            Some(
                "https://books.google.com/books/content?id=pD6arNyKyi8C&printsec=frontcover&img=1&zoom=1&source=gbs_api"
            )
        );
        let bare = &c[1];
        assert_eq!(bare.meta.published, "", "two-digit year is unusable");
        assert!(bare.cover_url.is_none());
        let u = &c[2];
        assert_eq!(u.meta.title, "Ünïcödé 日本");
        assert_eq!(u.meta.authors, vec!["A", "B", "C"]);
        assert_eq!(u.meta.published, "2003-05");
    }

    #[test]
    fn google_no_items_error_object_and_garbage() {
        assert!(
            GoogleBooks::parse_volumes(br#"{"kind":"books#volumes","totalItems":0}"#)
                .unwrap()
                .is_empty()
        );
        let e = GoogleBooks::parse_volumes(
            br#"{"error": {"code": 403, "message": "Daily Limit Exceeded"}}"#,
        )
        .unwrap_err();
        assert!(e.to_string().contains("Daily Limit Exceeded"), "{e}");
        let e = GoogleBooks::parse_volumes(br#"{"error": {}}"#).unwrap_err();
        assert!(e.to_string().contains("API error"));
        assert!(GoogleBooks::parse_volumes(b"<!doctype html>").is_err());
    }

    #[test]
    fn google_lookup_and_cover_via_mock() {
        let f = MockFetcher::new()
            .on_prefix("https://www.googleapis.com/books/v1/volumes", MockReply::Body(GB.as_bytes().to_vec()))
            .with_body("https://books.google.com/books/content?id=pD6arNyKyi8C&printsec=frontcover&img=1&zoom=1&source=gbs_api", jpeg(1500));
        let g = GoogleBooks::new(Arc::new(f), None);
        let found = MetadataSource::lookup(&g, &q("The Hobbit", &[])).unwrap();
        assert_eq!(found.len(), 3);
        assert_eq!(MetadataSource::id(&g), "google");
        assert_eq!(
            MetadataSource::cover(&g, &found[0])
                .unwrap()
                .map(|b| b.len()),
            Some(1500)
        );
        assert_eq!(MetadataSource::cover(&g, &found[1]).unwrap(), None);
    }

    #[test]
    fn google_http_errors() {
        for reply in [
            MockReply::Status(429),
            MockReply::Timeout,
            MockReply::Failure("tls".into()),
        ] {
            let g = GoogleBooks::new(
                Arc::new(MockFetcher::new().on_prefix("https://www.googleapis.com/", reply)),
                None,
            );
            assert!(g.candidates(&q("x", &[])).is_err());
        }
    }

    const WD: &str = r#"{
      "head": {"vars": ["item"]},
      "results": {"bindings": [
        {"item": {"type": "uri", "value": "http://www.wikidata.org/entity/Q74287"},
         "itemLabel": {"type": "literal", "value": "The Hobbit"},
         "authorLabel": {"value": "J. R. R. Tolkien"},
         "publisherLabel": {"value": "George Allen & Unwin"},
         "pub": {"value": "1937-09-21T00:00:00Z"},
         "seriesLabel": {"value": "Middle-earth"},
         "genreLabel": {"value": "fantasy"},
         "isbn13": {"value": "978-0-618-26030-0"}},
        {"item": {"value": "http://www.wikidata.org/entity/Q74287"},
         "itemLabel": {"value": "The Hobbit"},
         "authorLabel": {"value": "J. R. R. Tolkien"},
         "genreLabel": {"value": "children's literature"},
         "isbn10": {"value": "0-618-26030-7"}},
        {"item": {"value": "http://www.wikidata.org/entity/Q999"}, "itemLabel": {"value": "Q999"}},
        {"item": {"value": "http://www.wikidata.org/entity/Q5"}, "itemLabel": {"value": "Zażółć"},
         "authorLabel": {"value": "Ś"}, "pub": {"value": "garbage"}},
        {"itemLabel": {"value": "no item"}}
      ]}
    }"#;

    #[test]
    fn wikidata_sparql_building_escapes_input() {
        let s = Wikidata::sparql(&q("Say \"hi\" \\ }\nSELECT", &[]));
        assert!(
            s.contains(r#"mwapi:search "Say \"hi\" \\ } SELECT""#),
            "{s}"
        );
        assert!(!s.contains('\n'));
        let by_isbn = Wikidata::sparql(&MetadataQuery {
            isbn: Some("0-306-40615-2".into()),
            title: "x".into(),
            ..Default::default()
        });
        assert!(by_isbn.contains("= \"0306406152\""), "{by_isbn}");
        assert!(by_isbn.contains("P957"));
        let r = Wikidata::search_request(&q("Dune", &[]));
        assert!(
            r.url
                .starts_with("https://query.wikidata.org/sparql?format=json&query=SELECT%20")
        );
        assert_eq!(
            r.header_value("accept"),
            Some("application/sparql-results+json")
        );
    }

    #[test]
    fn wikidata_parsing_groups_rows_per_item() {
        let c = Wikidata::parse_sparql(WD.as_bytes()).unwrap();
        assert_eq!(
            c.len(),
            2,
            "unlabelled items and rows without item are dropped"
        );
        let h = &c[0].meta;
        assert_eq!(h.title, "The Hobbit");
        assert_eq!(h.authors, vec!["J. R. R. Tolkien"]);
        assert_eq!(h.publisher, "George Allen & Unwin");
        assert_eq!(h.published, "1937-09-21");
        assert_eq!(h.series, "Middle-earth");
        assert_eq!(h.tags, vec!["fantasy", "children's literature"]);
        let isbns: Vec<&str> = h
            .identifiers
            .iter()
            .filter(|i| i.scheme == "isbn")
            .map(|i| i.value.as_str())
            .collect();
        assert_eq!(isbns, vec!["9780618260300", "0618260307"]);
        assert!(
            h.identifiers
                .iter()
                .any(|i| i.scheme == "wikidata" && i.value == "Q74287")
        );
        assert_eq!(c[1].meta.title, "Zażółć");
        assert_eq!(c[1].meta.published, "");
        assert!(c[0].cover_url.is_none());
    }

    #[test]
    fn wikidata_errors() {
        assert!(
            Wikidata::parse_sparql(b"{}")
                .unwrap_err()
                .to_string()
                .contains("bindings")
        );
        assert!(Wikidata::parse_sparql(b"oops").is_err());
        assert!(
            Wikidata::parse_sparql(br#"{"results":{"bindings":[]}}"#)
                .unwrap()
                .is_empty()
        );
        let w = Wikidata::new(Arc::new(
            MockFetcher::new().on_prefix("https://query.wikidata.org/", MockReply::Status(504)),
        ));
        assert!(w.candidates(&q("Dune", &[])).is_err());
    }

    #[test]
    fn wikidata_lookup_through_trait() {
        let w = Wikidata::new(Arc::new(MockFetcher::new().on_prefix(
            "https://query.wikidata.org/",
            MockReply::Body(WD.as_bytes().to_vec()),
        )));
        let r = MetadataSource::lookup(&w, &q("The Hobbit", &[])).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(MetadataSource::id(&w), "wikidata");
        assert_eq!(MetadataSource::cover(&w, &r[0]).unwrap(), None);
    }

    // ---- ranking & merging ----

    fn cand(source: &str, title: &str, authors: &[&str]) -> Candidate {
        Candidate {
            meta: Metadata {
                title: title.into(),
                authors: authors.iter().map(|s| s.to_string()).collect(),
                ..Default::default()
            },
            cover_url: None,
            source: source.into(),
        }
    }

    struct Fake {
        id: &'static str,
        result: std::result::Result<Vec<Candidate>, String>,
    }

    impl CandidateSource for Fake {
        fn id(&self) -> &'static str {
            self.id
        }
        fn candidates(&self, _: &MetadataQuery) -> Result<Vec<Candidate>> {
            self.result.clone().map_err(|e| anyhow!(e))
        }
    }

    fn set(sources: Vec<Fake>) -> SourceSet {
        let mut s = SourceSet::new(Arc::new(MockFetcher::new()));
        for f in sources {
            s.add(Box::new(f));
        }
        s
    }

    #[test]
    fn author_scores() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(author_score(&[], &a(&["X"])), 1.0);
        assert_eq!(
            author_score(&a(&["J.R.R. Tolkien"]), &a(&["Tolkien, John Ronald Reuel"])),
            1.0
        );
        assert_eq!(author_score(&a(&["Tolkien"]), &[]), 0.5);
        assert!(
            author_score(&a(&["Tolkein"]), &a(&["Tolkien"])) > 0.8,
            "typo tolerant"
        );
        assert!(author_score(&a(&["Tolkien"]), &a(&["Rowling"])) < 0.6);
        let two = author_score(
            &a(&["Neil Gaiman", "Terry Pratchett"]),
            &a(&["Neil Gaiman"]),
        );
        assert!(two > 0.5 && two < 0.9, "{two}");
    }

    #[test]
    fn candidate_scoring() {
        let query = q("The Hobbit", &["J.R.R. Tolkien"]);
        let exact = score_candidate(&query, &cand("a", "The Hobbit", &["Tolkien, J. R. R."]));
        let wrong_author = score_candidate(&query, &cand("a", "The Hobbit", &["Someone Else"]));
        let wrong_title = score_candidate(&query, &cand("a", "Neuromancer", &["J.R.R. Tolkien"]));
        assert!(exact > 0.99, "{exact}");
        assert!(wrong_author < exact && wrong_title < exact);
        let title_only = score_candidate(&q("The Hobbit", &[]), &cand("a", "the hobbit", &[]));
        assert_eq!(title_only, 1.0);
        let with_isbn = MetadataQuery {
            title: "zzz".into(),
            isbn: Some("0306406152".into()),
            ..Default::default()
        };
        let mut c = cand("a", "Whatever", &[]);
        c.meta.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: "978-0-306-40615-7".into(),
            xml_id: None,
        });
        assert_eq!(
            score_candidate(&with_isbn, &c),
            1.0,
            "ISBN-10 query matches ISBN-13 record"
        );
        assert_eq!(score_candidate(&MetadataQuery::default(), &c), 0.5);
    }

    #[test]
    fn source_set_ranks_and_merges_by_priority() {
        let mut ol = cand("ol", "The Hobbit", &["J.R.R. Tolkien"]);
        ol.meta.publisher = "Allen & Unwin".into();
        ol.meta.tags = vec!["Fantasy".into()];
        ol.meta.description = "short".into();
        ol.cover_url = Some("https://ol/cover".into());
        let mut gb = cand("gb", "The Hobbit", &["J. R. R. Tolkien"]);
        gb.meta.publisher = "Houghton".into();
        gb.meta.description = "long description".into();
        gb.meta.tags = vec!["fantasy".into(), "Classic".into()];
        gb.meta.language = "en".into();
        gb.cover_url = Some("https://gb/cover".into());
        let junk = cand("ol", "Cooking for Dummies", &["Nobody"]);
        let s = set(vec![
            Fake {
                id: "ol",
                result: Ok(vec![junk, ol]),
            },
            Fake {
                id: "gb",
                result: Ok(vec![gb]),
            },
        ])
        .with_priority(FieldPriority::default().prefer(Field::Description, &["gb"]));
        let r = s.search(&q("The Hobbit", &["J.R.R. Tolkien"])).unwrap();
        assert_eq!(r.ranked.len(), 3);
        assert!(r.ranked[0].score >= r.ranked[1].score && r.ranked[1].score >= r.ranked[2].score);
        assert_eq!(r.ranked[2].candidate.meta.title, "Cooking for Dummies");
        let best = r.best.unwrap();
        assert_eq!(best.sources, vec!["ol", "gb"]);
        assert_eq!(
            best.meta.publisher, "Allen & Unwin",
            "default order: first source wins"
        );
        assert_eq!(
            best.meta.description, "long description",
            "priority override"
        );
        assert_eq!(
            best.meta.language, "en",
            "gaps filled from the other source"
        );
        assert_eq!(
            best.meta.tags,
            vec!["Fantasy", "Classic"],
            "tags unioned case-insensitively"
        );
        assert_eq!(best.cover_url.as_deref(), Some("https://ol/cover"));
        assert!(
            best.confidence > 0.99 && best.confidence <= 1.0,
            "{}",
            best.confidence
        );
        assert!(r.errors.is_empty());
        let fs = &best.field_sources;
        assert_eq!(fs["publisher"], "ol");
        assert_eq!(fs["description"], "gb", "the priority override is recorded");
        assert_eq!(fs["language"], "gb", "a gap filled by the second source");
        assert_eq!(fs["tags"], "ol, gb");
        assert_eq!(fs["cover"], "ol");
        assert_eq!(fs["authors"], "ol");
    }

    #[test]
    fn source_set_series_and_identifiers_merge() {
        let mut a = cand("a", "Dune", &["Frank Herbert"]);
        a.meta.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: "0441172717".into(),
            xml_id: None,
        });
        let mut b = cand("b", "Dune", &["Frank Herbert"]);
        b.meta.series = "Dune Chronicles".into();
        b.meta.series_index = 1.0;
        b.meta.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: "0441172717".into(),
            xml_id: None,
        });
        b.meta.identifiers.push(Identifier {
            scheme: "google".into(),
            value: "g".into(),
            xml_id: None,
        });
        let r = set(vec![
            Fake {
                id: "a",
                result: Ok(vec![a]),
            },
            Fake {
                id: "b",
                result: Ok(vec![b]),
            },
        ])
        .search(&q("Dune", &["Frank Herbert"]))
        .unwrap();
        let m = r.best.unwrap().meta;
        assert_eq!(
            (m.series.as_str(), m.series_index),
            ("Dune Chronicles", 1.0)
        );
        assert_eq!(m.identifiers.len(), 2);
    }

    #[test]
    fn source_set_low_confidence_returns_no_best() {
        let s = set(vec![Fake {
            id: "a",
            result: Ok(vec![cand("a", "Completely Unrelated Cookbook", &["Chef"])]),
        }]);
        let r = s.search(&q("Quantum Field Theory", &["Weinberg"])).unwrap();
        assert!(r.best.is_none());
        assert_eq!(r.ranked.len(), 1);
    }

    #[test]
    fn source_set_no_candidates_is_ok_but_empty() {
        let r = set(vec![Fake {
            id: "a",
            result: Ok(vec![]),
        }])
        .search(&q("x", &[]))
        .unwrap();
        assert!(r.best.is_none() && r.ranked.is_empty());
    }

    #[test]
    fn source_set_partial_failure_is_reported() {
        let s = set(vec![
            Fake {
                id: "bad",
                result: Err("HTTP error 500".into()),
            },
            Fake {
                id: "good",
                result: Ok(vec![cand("good", "Dune", &["Frank Herbert"])]),
            },
        ]);
        let r = s.search(&q("Dune", &["Frank Herbert"])).unwrap();
        assert_eq!(
            r.errors,
            vec![SourceError {
                source: "bad".into(),
                message: "HTTP error 500".into()
            }]
        );
        assert_eq!(r.best.unwrap().sources, vec!["good"]);
    }

    #[test]
    fn source_set_total_failure_and_empty_set_are_errors() {
        let s = set(vec![
            Fake {
                id: "a",
                result: Err("timeout".into()),
            },
            Fake {
                id: "b",
                result: Err("bad json".into()),
            },
        ]);
        let e = s.search(&q("x", &[])).unwrap_err().to_string();
        assert!(
            e.contains("all metadata sources failed")
                && e.contains("a: timeout")
                && e.contains("b: bad json"),
            "{e}"
        );
        let e = set(vec![]).search(&q("x", &[])).unwrap_err().to_string();
        assert!(e.contains("no metadata sources"));
        assert!(set(vec![]).is_empty());
    }

    #[test]
    fn source_set_isbn_query_prefers_matching_isbn() {
        let mut right = cand("a", "A Different Printing Title", &[]);
        right.meta.identifiers.push(Identifier {
            scheme: "isbn".into(),
            value: "9780306406157".into(),
            xml_id: None,
        });
        let other = cand("a", "Totally Other", &[]);
        let query = MetadataQuery {
            isbn: Some("0-306-40615-2".into()),
            ..Default::default()
        };
        let r = set(vec![Fake {
            id: "a",
            result: Ok(vec![other, right]),
        }])
        .search(&query)
        .unwrap();
        assert_eq!(
            r.ranked[0].candidate.meta.title,
            "A Different Printing Title"
        );
        assert_eq!(r.best.unwrap().confidence, 1.0);
    }

    #[test]
    fn standard_set_end_to_end_with_mocks() {
        let f = MockFetcher::new()
            .on_prefix(
                "https://openlibrary.org/search.json",
                MockReply::Body(OL_SEARCH.as_bytes().to_vec()),
            )
            .on_prefix(
                "https://www.googleapis.com/",
                MockReply::Body(GB.as_bytes().to_vec()),
            )
            .on_prefix("https://query.wikidata.org/", MockReply::Status(500))
            .with_body(
                "https://covers.openlibrary.org/b/id/8406786-L.jpg",
                jpeg(3000),
            );
        let set = SourceSet::standard(Arc::new(f));
        assert_eq!(set.len(), 3);
        let r = set.search(&q("The Hobbit", &["J.R.R. Tolkien"])).unwrap();
        assert_eq!(r.errors.len(), 1);
        assert_eq!(r.errors[0].source, "wikidata");
        let best = r.best.unwrap();
        assert!(
            best.sources.contains(&"openlibrary".to_string())
                && best.sources.contains(&"google".to_string())
        );
        assert!(
            best.meta.description.starts_with("In a hole"),
            "Google preferred for descriptions"
        );
        assert_eq!(best.meta.series, "Middle-earth");
        assert_eq!(set.fetch_cover(&best).unwrap().map(|b| b.len()), Some(3000));
        let none = Best {
            meta: Metadata::default(),
            cover_url: None,
            confidence: 1.0,
            sources: vec![],
            field_sources: Default::default(),
        };
        assert_eq!(set.fetch_cover(&none).unwrap(), None);
    }

    #[test]
    fn source_set_as_metadata_source() {
        let f = MockFetcher::new()
            .on_prefix(
                "https://openlibrary.org/search.json",
                MockReply::Body(OL_SEARCH.as_bytes().to_vec()),
            )
            .on_prefix("https://www.googleapis.com/", MockReply::Status(500))
            .on_prefix("https://query.wikidata.org/", MockReply::Status(500))
            .with_body(
                "https://covers.openlibrary.org/b/id/8406786-L.jpg",
                jpeg(3000),
            );
        let set = SourceSet::standard(Arc::new(f));
        let list = MetadataSource::lookup(&set, &q("The Hobbit", &["Tolkien"])).unwrap();
        assert_eq!(list.len(), 4, "merged best + 3 candidates");
        assert!(list[0].title.starts_with("The Hobbit"));
        assert_eq!(MetadataSource::id(&set), "sources");
        let cover = MetadataSource::cover(&set, &list[0]).unwrap();
        assert_eq!(cover.map(|b| b.len()), Some(3000));
        let nothing = MetadataSource::cover(
            &set,
            &Metadata {
                title: "Quantum Gravity Primer".into(),
                ..Default::default()
            },
        );
        assert!(nothing.is_ok());
    }

    #[test]
    fn user_priority_overlays_the_defaults_and_ignores_nonsense() {
        let mut user = std::collections::BTreeMap::new();
        user.insert("description".to_string(), vec!["openlibrary".to_string()]);
        user.insert("tags".to_string(), vec!["nosuchsource".to_string()]);
        user.insert("nosuchfield".to_string(), vec!["google".to_string()]);
        let p = SourceSet::standard_priority().overlay(&user);
        assert_eq!(p.preferred(Field::Description), Some("openlibrary"));
        assert_eq!(p.preferred(Field::Series), Some("wikidata"), "default kept");
        assert_eq!(
            p.preferred(Field::Tags),
            None,
            "an invalid choice is dropped"
        );
        for f in Field::ALL {
            assert_eq!(Field::from_key(f.key()), Some(f));
        }
    }

    #[test]
    fn field_priority_ranking() {
        let p = FieldPriority::default().prefer(Field::Tags, &["b", "a"]);
        assert!(p.rank(Field::Tags, "b", 5) < p.rank(Field::Tags, "a", 0));
        assert!(p.rank(Field::Tags, "a", 0) < p.rank(Field::Tags, "c", 0));
        assert!(
            p.rank(Field::Title, "x", 0) < p.rank(Field::Title, "y", 1),
            "default order by source position"
        );
    }
}
