//! The tools: their published definitions (JSON Schemas and annotations), the call path (rate
//! limit, argument check, audit of write tools) and the read-only tools.

use std::sync::Arc;
use std::time::Instant;

use bookx_core::library::{Book, SortKey};
use bookx_core::query::Query;
use serde_json::{json, Value};

use crate::app::{fail, redact, App, Config, ToolError, ToolOutput};
use crate::schema;
use crate::server::{decode_cursor, encode_cursor, Ctx, RpcError, LOG_LEVELS};
use crate::tools_write;

/// An answer larger than this (bytes of JSON) is refused instead of sent.
pub const MAX_OUTPUT: usize = 256 * 1024;
pub const COPYRIGHT_NOTE: &str = "Text of the user's own copy, for personal use. Quote briefly; do not reproduce whole chapters.";

#[derive(Debug, Clone)]
pub struct ToolDef {
    pub name: &'static str,
    pub title: &'static str,
    pub description: String,
    pub input: Value,
    pub output: Value,
    pub write: bool,
    pub destructive: bool,
    pub idempotent: bool,
    pub open_world: bool,
}

impl ToolDef {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "title": self.title,
            "description": self.description,
            "inputSchema": self.input,
            "outputSchema": self.output,
            "annotations": {
                "title": self.title,
                "readOnlyHint": !self.write,
                "destructiveHint": self.write && self.destructive,
                "idempotentHint": !self.write || self.idempotent,
                "openWorldHint": self.open_world
            }
        })
    }
}

fn paging(mut props: Value) -> Value {
    props["limit"] = json!({"type":"integer","minimum":1,"maximum":100,"description":"Most items to return (default 20)."});
    props["cursor"] = json!({"type":"string","description":"The next_cursor of the previous answer, to get the next page."});
    props
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}

fn out(props: Value) -> Value {
    json!({"type":"object","properties":props})
}

fn read(
    name: &'static str,
    title: &'static str,
    description: &str,
    input: Value,
    output: Value,
) -> ToolDef {
    ToolDef {
        name,
        title,
        description: description.into(),
        input,
        output,
        write: false,
        destructive: false,
        idempotent: true,
        open_world: false,
    }
}

pub(crate) fn write_def(
    name: &'static str,
    title: &'static str,
    description: &str,
    input: Value,
    idempotent: bool,
) -> ToolDef {
    ToolDef {
        name,
        title,
        description: description.into(),
        input,
        output: out(json!({"ok":{"type":"boolean"},"message":{"type":"string"}})),
        write: true,
        destructive: false,
        idempotent,
        open_world: false,
    }
}

const BOOK_ID: &str = "Id of the book (from search_books).";

fn book_id() -> Value {
    json!({"type":"integer","minimum":1,"description":BOOK_ID})
}

/// Every tool this configuration offers. Write tools are absent unless writing is allowed.
pub fn defs(cfg: &Config) -> Vec<ToolDef> {
    let mut v = vec![
        read("search_books", "Search books",
            "Search the library with the Bookx query language: free words match title/author/tags; fields are title:, author:, tag:, series:, publisher:, language:, format:, identifier:/isbn:, comments:, rating:>=4 (stars), progress:>50, status:new|reading|finished, size:, drm:, weak:; `=` after the colon means exact (author:=\"Ann Lee\"), `-` negates, OR joins groups. Returns book summaries, newest page first by the chosen sort.",
            obj(paging(json!({
                "query":{"type":"string","maxLength":500,"description":"Search expression; empty lists everything."},
                "sort":{"type":"string","enum":["title","author","added","last_read","rating","series","progress","size","published"],"description":"Default title."},
                "descending":{"type":"boolean"}})), &[]),
            out(json!({"total":{"type":"integer"},"books":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("get_book", "Get one book",
            "Full metadata of a book: authors, series, tags, rating (stars), description, identifiers, formats, where each field came from (provenance), reading progress and status, shelves, counts of highlights and bookmarks.",
            obj(json!({"book_id":book_id()}), &["book_id"]),
            out(json!({"book":{"type":"object"}}))),
        read("list_collections", "List shelves",
            "Lists the shelves (collections) with their book counts; with collection_id lists the books on that shelf.",
            obj(paging(json!({"collection_id":{"type":"integer","description":"List the books of this shelf."}})), &[]),
            out(json!({"collections":{"type":"array"},"books":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("list_series", "List series",
            "Series in the library with how many books each has and their first volumes.",
            obj(paging(json!({"query":{"type":"string","maxLength":200},"sort":{"type":"string","enum":["name","count"]}})), &[]),
            out(json!({"total":{"type":"integer"},"series":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("list_authors", "List authors",
            "Authors in the library with their number of books.",
            obj(paging(json!({"query":{"type":"string","maxLength":200},"sort":{"type":"string","enum":["name","count"]}})), &[]),
            out(json!({"total":{"type":"integer"},"authors":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("list_tags", "List tags",
            "Tags in the library with their number of books.",
            obj(paging(json!({"query":{"type":"string","maxLength":200},"sort":{"type":"string","enum":["name","count"]}})), &[]),
            out(json!({"total":{"type":"integer"},"tags":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("get_toc", "Table of contents",
            "The table of contents of a book and its chapters (index, title, length). Chapter indexes are 0-based and are what read_book_text and search_in_book use.",
            obj(json!({"book_id":book_id()}), &["book_id"]),
            out(json!({"toc":{"type":"array"},"chapters":{"type":"array"}}))),
        read("read_book_text", "Read book text",
            "Plain text of a book, one piece at a time (at most max_chars characters). Give chapter (0-based) and optionally offset, or pass the next_cursor of the last answer to continue, also across chapters. Copy-protected books are refused.",
            obj(json!({"book_id":book_id(),
                "chapter":{"type":"integer","minimum":0,"description":"0-based chapter index; default 0."},
                "offset":{"type":"integer","minimum":0,"description":"Character offset inside the chapter."},
                "max_chars":{"type":"integer","minimum":200,"maximum":20000,"description":"Default 8000."},
                "cursor":{"type":"string","description":"next_cursor of the previous answer (overrides chapter and offset)."}}), &["book_id"]),
            out(json!({"text":{"type":"string"},"chapter":{"type":"integer"},"next_cursor":{"type":"string"}}))),
        read("search_in_book", "Search inside a book",
            "Finds words or a phrase (or a regular expression) inside one book and returns snippets with their chapter and offset.",
            obj(paging(json!({"book_id":book_id(),
                "query":{"type":"string","minLength":1,"maxLength":300},
                "whole_word":{"type":"boolean"},"case_sensitive":{"type":"boolean"},"regex":{"type":"boolean"}})), &["book_id","query"]),
            out(json!({"hits":{"type":"array"},"truncated":{"type":"boolean"},"next_cursor":{"type":"string"}}))),
        read("search_all_books_text", "Search the text of all books",
            "Full-text search over the indexed text of every book (stemmed; every word must occur). Returns the books and a snippet each. Books whose text is not indexed yet (the app indexes while idle) are not found.",
            obj(paging(json!({"query":{"type":"string","minLength":1,"maxLength":300}})), &["query"]),
            out(json!({"hits":{"type":"array"},"indexed_books":{"type":"integer"},"next_cursor":{"type":"string"}}))),
        read("get_annotations", "Highlights and notes",
            "Highlights, notes and bookmarks of one book or of all books, newest first.",
            obj(paging(json!({"book_id":book_id(),
                "kind":{"type":"string","enum":["all","highlight","note","bookmark"],"description":"note = highlights that carry a note."},
                "color":{"type":"string"},"query":{"type":"string","maxLength":200,"description":"Only annotations containing this text."}})), &[]),
            out(json!({"total":{"type":"integer"},"annotations":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("export_annotations", "Export annotations as text",
            "The highlights, notes and bookmarks of a book as Markdown, plain text or JSON (returned, nothing is written to disk).",
            obj(json!({"book_id":book_id(),"format":{"type":"string","enum":["markdown","text","json"]}}), &["book_id"]),
            out(json!({"format":{"type":"string"},"content":{"type":"string"}}))),
        read("get_reading_stats", "Reading statistics",
            "How much the user reads: library counts by status and, when the app recorded sessions, time, words and pages read in the last days, streak and per-book totals.",
            obj(json!({"days":{"type":"integer","minimum":1,"maximum":3650,"description":"Window for the totals; default 30."}}), &[]),
            out(json!({"library":{"type":"object"},"sessions":{"type":"object"}}))),
        read("recent_books", "Recent books",
            "Books last read, or last added.",
            obj(json!({"kind":{"type":"string","enum":["read","added"]},"limit":{"type":"integer","minimum":1,"maximum":100}}), &[]),
            out(json!({"books":{"type":"array"}}))),
        read("find_duplicates", "Find duplicates",
            "Groups of books that are probably the same work (same ISBN, or very similar title and author).",
            obj(paging(json!({"threshold":{"type":"number","minimum":0.5,"maximum":1,"description":"Similarity needed; default 0.9."}})), &[]),
            out(json!({"total":{"type":"integer"},"groups":{"type":"array"},"next_cursor":{"type":"string"}}))),
        read("similar_books", "Similar books",
            "Books like this one by shared author, series, tags, language and publisher, with the reasons.",
            obj(json!({"book_id":book_id(),"limit":{"type":"integer","minimum":1,"maximum":50}}), &["book_id"]),
            out(json!({"books":{"type":"array"}}))),
        read("check_book", "Check a book",
            "Runs the EPUB validator and returns the counts and the first problems (broken links, missing files, bad metadata).",
            obj(json!({"book_id":book_id()}), &["book_id"]),
            out(json!({"errors":{"type":"integer"},"warnings":{"type":"integer"},"issues":{"type":"array"}}))),
        read("convert_preview", "Preview a conversion",
            "What converting the book to another format would lose (images, cover, table of contents, links, series), without converting.",
            obj(json!({"book_id":book_id(),"format":{"type":"string","minLength":2,"maxLength":10,"description":"Target extension such as mobi, fb2, txt, kepub."}}), &["book_id","format"]),
            out(json!({"format":{"type":"string"},"lost":{"type":"array"}}))),
    ];
    if cfg.can_write() {
        v.extend(tools_write::defs(cfg));
    }
    v
}

// -- the call path --------------------------------------------------------------------------------

fn text_result(text: &str, structured: Option<Value>, is_error: bool) -> Value {
    let mut r = json!({"content":[{"type":"text","text":text}],"isError":is_error});
    if let Some(s) = structured {
        r["structuredContent"] = s;
    }
    r
}

pub fn call(
    app: &Arc<App>,
    name: &str,
    args: &Value,
    ctx: &Ctx,
    log_level: usize,
) -> Result<Value, RpcError> {
    let def = defs(&app.cfg)
        .into_iter()
        .find(|d| d.name == name)
        .ok_or_else(|| RpcError::invalid(format!("Unknown tool: {name}")))?;
    if app.is_stopped() {
        return Ok(text_result(
            "the Bookx MCP server has been switched off",
            None,
            true,
        ));
    }
    if let Err(wait) = app.limiter.check(Instant::now()) {
        return Ok(text_result(
            &format!("rate limit reached; try again in {} s", wait.as_secs() + 1),
            None,
            true,
        ));
    }
    if let Err(e) = schema::validate(&def.input, args) {
        return Ok(text_result(&format!("invalid arguments: {e}"), None, true));
    }
    let result = if def.write {
        tools_write::call(app, name, args, ctx)
    } else {
        dispatch(app, name, args, ctx)
    };
    if def.write {
        let (ok, summary) = match &result {
            Ok(o) => (
                true,
                o.structured
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("done")
                    .to_string(),
            ),
            Err(e) => (false, e.0.clone()),
        };
        app.audit.record(ctx.client, name, args, ok, &summary);
        if log_level <= 1 {
            (ctx.notify)(
                json!({"jsonrpc":"2.0","method":"notifications/message","params":{
                "level": LOG_LEVELS[1], "logger": "bookx",
                "data": {"tool": name, "ok": ok, "summary": summary, "arguments": redact(args)}}}),
            );
        }
    }
    Ok(match result {
        Ok(o) => {
            let text = o
                .text
                .unwrap_or_else(|| serde_json::to_string_pretty(&o.structured).unwrap_or_default());
            if text.len() > MAX_OUTPUT || o.structured.to_string().len() > MAX_OUTPUT {
                text_result(
                    "the answer is too large; ask for fewer items (limit) or a smaller piece",
                    None,
                    true,
                )
            } else {
                text_result(&text, Some(o.structured), false)
            }
        }
        Err(e) => {
            let msg = if ctx.timed_out() {
                format!("stopped after the time limit: {}", app.scrub(&e.0))
            } else {
                app.scrub(&e.0)
            };
            text_result(&msg, None, true)
        }
    })
}

fn dispatch(app: &Arc<App>, name: &str, a: &Value, ctx: &Ctx) -> Result<ToolOutput, ToolError> {
    match name {
        "search_books" => search_books(app, a),
        "get_book" => get_book(app, a),
        "list_collections" => list_collections(app, a),
        "list_series" => facet(app, a, "series"),
        "list_authors" => facet(app, a, "authors"),
        "list_tags" => facet(app, a, "tags"),
        "get_toc" => get_toc(app, a),
        "read_book_text" => read_book_text(app, a),
        "search_in_book" => search_in_book(app, a, ctx),
        "search_all_books_text" => search_all(app, a),
        "get_annotations" => get_annotations(app, a, ctx),
        "export_annotations" => export_annotations(app, a),
        "get_reading_stats" => reading_stats(app, a),
        "recent_books" => recent_books(app, a),
        "find_duplicates" => find_duplicates(app, a, ctx),
        "similar_books" => similar_books(app, a),
        "check_book" => check_book(app, a),
        "convert_preview" => convert_preview(app, a),
        other => fail(format!("unknown tool {other}")),
    }
}

// -- argument helpers --------------------------------------------------------------------------------

pub(crate) fn arg_id(a: &Value) -> Result<u64, ToolError> {
    a.get("book_id")
        .and_then(Value::as_u64)
        .ok_or_else(|| ToolError("`book_id` is required".into()))
}

pub(crate) fn arg_str<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}

fn arg_bool(a: &Value, k: &str) -> bool {
    a.get(k).and_then(Value::as_bool).unwrap_or(false)
}

/// `(offset, limit)` of a paged tool call.
fn page_args(a: &Value) -> Result<(usize, usize), ToolError> {
    let limit = a.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
    let offset = match arg_str(a, "cursor") {
        Some(c) => decode_cursor(c).map_err(|e| ToolError(e.message))?,
        None => 0,
    };
    Ok((offset, limit.clamp(1, 100)))
}

/// The page `[offset, offset+limit)` of `items` and the cursor of the next one.
fn page<T: Clone>(items: &[T], offset: usize, limit: usize) -> (Vec<T>, Option<String>) {
    let start = offset.min(items.len());
    let end = (start + limit).min(items.len());
    (
        items[start..end].to_vec(),
        (end < items.len()).then(|| encode_cursor(end)),
    )
}

fn with_next(mut v: Value, next: Option<String>) -> Value {
    if let Some(n) = next {
        v["next_cursor"] = json!(n);
    }
    v
}

fn cut(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        format!("{}…", s.chars().take(n).collect::<String>())
    }
}

pub fn stars(rating: u8) -> f64 {
    f64::from(rating) / 2.0
}

pub fn summary(b: &Book) -> Value {
    json!({
        "id": b.id,
        "title": b.meta.title,
        "authors": b.meta.authors,
        "series": b.meta.series,
        "series_index": b.meta.series_index,
        "tags": b.meta.tags,
        "rating": stars(b.meta.rating),
        "language": b.meta.language,
        "status": b.status(),
        "progress": (b.progress * 1000.0).round() / 1000.0,
        "formats": b.all_formats().iter().map(|f| f.format.clone()).collect::<Vec<_>>(),
        "has_cover": b.cover.is_some(),
        "protected": !b.drm.is_empty(),
    })
}

fn sort_books(v: &mut [&Book], key: SortKey, asc: bool) {
    use std::cmp::Ordering::Equal;
    v.sort_by(|a, b| {
        let o = match key {
            SortKey::Title => a
                .meta
                .title_sort
                .to_lowercase()
                .cmp(&b.meta.title_sort.to_lowercase()),
            SortKey::Author => a
                .meta
                .author_sort
                .to_lowercase()
                .cmp(&b.meta.author_sort.to_lowercase()),
            SortKey::DateAdded => a.added.cmp(&b.added),
            SortKey::LastRead => a.last_read.cmp(&b.last_read),
            SortKey::Published => a.meta.published.cmp(&b.meta.published),
            SortKey::Rating => a.meta.rating.cmp(&b.meta.rating),
            SortKey::Series => a
                .meta
                .series
                .to_lowercase()
                .cmp(&b.meta.series.to_lowercase())
                .then(
                    a.meta
                        .series_index
                        .partial_cmp(&b.meta.series_index)
                        .unwrap_or(Equal),
                ),
            SortKey::Size => a.size.cmp(&b.size),
            SortKey::Progress => a.progress.partial_cmp(&b.progress).unwrap_or(Equal),
        };
        let o = o.then_with(|| {
            a.meta
                .title_sort
                .to_lowercase()
                .cmp(&b.meta.title_sort.to_lowercase())
        });
        if asc {
            o
        } else {
            o.reverse()
        }
    });
}

fn sort_key(name: &str) -> SortKey {
    match name {
        "author" => SortKey::Author,
        "added" => SortKey::DateAdded,
        "last_read" => SortKey::LastRead,
        "rating" => SortKey::Rating,
        "series" => SortKey::Series,
        "progress" => SortKey::Progress,
        "size" => SortKey::Size,
        "published" => SortKey::Published,
        _ => SortKey::Title,
    }
}

// -- the tools ---------------------------------------------------------------------------------------

fn search_books(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let books = app.catalog.books()?;
    let q = Query::parse(arg_str(a, "query").unwrap_or(""));
    let mut found: Vec<&Book> = books.iter().filter(|b| q.matches(b)).collect();
    sort_books(
        &mut found,
        sort_key(arg_str(a, "sort").unwrap_or("title")),
        !arg_bool(a, "descending"),
    );
    let (items, next) = page(&found, offset, limit);
    Ok(ToolOutput::json(with_next(
        json!({"total": found.len(), "books": items.iter().map(|b| summary(b)).collect::<Vec<_>>()}),
        next,
    )))
}

fn get_book(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let b = app.catalog.book(arg_id(a)?)?;
    Ok(ToolOutput::json(json!({"book": book_json(app, &b)?})))
}

/// The full description of a book (the `get_book` answer and the `bookx://book/{id}` resource).
pub fn book_json(app: &Arc<App>, b: &Book) -> Result<Value, ToolError> {
    let store = app.catalog.store()?;
    let mut v = summary(b);
    let m = &b.meta;
    v["description"] = json!(cut(&m.description, 4000));
    v["publisher"] = json!(m.publisher);
    v["published"] = json!(m.published);
    v["rights"] = json!(m.rights);
    v["identifiers"] = json!(m
        .identifiers
        .iter()
        .map(|i| json!({"scheme": i.scheme, "value": i.value}))
        .collect::<Vec<_>>());
    v["formats"] = json!(b
        .all_formats()
        .iter()
        .map(|f| json!({"format": f.format, "size": f.size,
            "name": f.file.rsplit('/').next().unwrap_or(&f.file)}))
        .collect::<Vec<_>>());
    v["size"] = json!(b.size);
    v["added"] = json!(b.added.to_rfc3339());
    v["modified"] = json!(b.modified.to_rfc3339());
    v["last_read"] = json!(b.last_read.map(|t| t.to_rfc3339()));
    v["position"] = json!(b
        .position
        .map(|p| json!({"chapter": p.spine, "fraction": p.frac})));
    v["highlights"] = json!(b.highlights.len());
    v["bookmarks"] = json!(b.bookmarks.len());
    v["protection"] = json!(b.drm);
    v["reading_status"] = json!(store.get_status(b.id).map(|s| s.as_str()));
    let shelves = store.list_collections().unwrap_or_default();
    let mine = store.collections_of(b.id);
    v["shelves"] = json!(shelves
        .iter()
        .filter(|c| mine.contains(&c.id))
        .map(|c| c.name.clone())
        .collect::<Vec<_>>());
    v["provenance"] = json!(store
        .provenance(b.id)
        .iter()
        .map(|p| json!({"field": p.field, "source": p.source, "at": p.at}))
        .collect::<Vec<_>>());
    Ok(v)
}

fn list_collections(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let store = app.catalog.store()?;
    if let Some(id) = a.get("collection_id").and_then(Value::as_i64) {
        let books = app.catalog.books()?;
        let ids = store.collection_books(id);
        let found: Vec<&Book> = ids
            .iter()
            .filter_map(|i| books.iter().find(|b| b.id == *i))
            .collect();
        let (items, next) = page(&found, offset, limit);
        return Ok(ToolOutput::json(with_next(
            json!({"collection_id": id, "total": found.len(),
                "books": items.iter().map(|b| summary(b)).collect::<Vec<_>>()}),
            next,
        )));
    }
    let all = store.list_collections()?;
    let (items, next) = page(&all, offset, limit);
    Ok(ToolOutput::json(with_next(
        json!({"total": all.len(), "collections": items.iter()
            .map(|c| json!({"id": c.id, "name": c.name, "books": c.books})).collect::<Vec<_>>()}),
        next,
    )))
}

fn facet(app: &Arc<App>, a: &Value, kind: &str) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let books = app.catalog.books()?;
    let mut map: std::collections::BTreeMap<String, Vec<&Book>> = Default::default();
    for b in books.iter() {
        match kind {
            "series" if !b.meta.series.is_empty() => {
                map.entry(b.meta.series.clone()).or_default().push(b)
            }
            "authors" => b
                .meta
                .authors
                .iter()
                .for_each(|x| map.entry(x.clone()).or_default().push(b)),
            "tags" => b
                .meta
                .tags
                .iter()
                .for_each(|x| map.entry(x.clone()).or_default().push(b)),
            _ => {}
        }
    }
    let needle = arg_str(a, "query").unwrap_or("").to_lowercase();
    let mut rows: Vec<(String, Vec<&Book>)> = map
        .into_iter()
        .filter(|(n, _)| needle.is_empty() || n.to_lowercase().contains(&needle))
        .collect();
    if arg_str(a, "sort") == Some("count") {
        rows.sort_by(|x, y| y.1.len().cmp(&x.1.len()).then(x.0.cmp(&y.0)));
    }
    let (items, next) = page(&rows, offset, limit);
    let rows_json: Vec<Value> = items
        .iter()
        .map(|(name, bs)| {
            let mut r = json!({"name": name, "books": bs.len()});
            if kind == "series" {
                let mut first = bs.clone();
                first.sort_by(|x, y| {
                    x.meta
                        .series_index
                        .partial_cmp(&y.meta.series_index)
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                r["volumes"] = json!(first
                    .iter()
                    .take(12)
                    .map(
                        |b| json!({"id": b.id, "title": b.meta.title, "index": b.meta.series_index})
                    )
                    .collect::<Vec<_>>());
            }
            r
        })
        .collect();
    Ok(ToolOutput::json(with_next(
        json!({"total": rows.len(), kind: rows_json}),
        next,
    )))
}

fn get_toc(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let b = app.catalog.book(arg_id(a)?)?;
    let t = app.texts.get(app.catalog.root(), &b)?;
    let chapters: Vec<Value> = t
        .chapters
        .iter()
        .map(|c| {
            json!({"index": c.index, "title": c.title, "chars": c.text.chars().count(),
            "words": c.text.split_whitespace().count()})
        })
        .collect();
    let toc: Vec<Value> = t
        .toc
        .iter()
        .take(500)
        .map(|(d, title, idx)| json!({"depth": d, "title": title, "chapter": idx}))
        .collect();
    Ok(ToolOutput::json(json!({
        "book_id": b.id, "title": b.meta.title,
        "toc": toc, "chapters": chapters.iter().take(1000).collect::<Vec<_>>(),
        "chapter_count": chapters.len()
    })))
}

fn read_book_text(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let b = app.catalog.book(arg_id(a)?)?;
    let t = app.texts.get(app.catalog.root(), &b)?;
    let max = (a.get("max_chars").and_then(Value::as_u64).unwrap_or(8000) as usize)
        .min(app.cfg.max_text_chars);
    let (mut chapter, mut offset) = (
        a.get("chapter").and_then(Value::as_u64).unwrap_or(0) as usize,
        a.get("offset").and_then(Value::as_u64).unwrap_or(0) as usize,
    );
    if let Some(c) = arg_str(a, "cursor") {
        let n = decode_cursor(c).map_err(|e| ToolError(e.message))?;
        chapter = n >> 32;
        offset = n & 0xffff_ffff;
    }
    let ch = t.chapters.get(chapter).ok_or_else(|| {
        ToolError(format!(
            "this book has {} chapters (0-{}); there is no chapter {chapter}",
            t.chapters.len(),
            t.chapters.len().saturating_sub(1)
        ))
    })?;
    let total = ch.text.chars().count();
    let (piece, next_off) = crate::text::slice_chars(&ch.text, offset, max);
    let next = match next_off {
        Some(o) => Some((chapter << 32) | o),
        None if chapter + 1 < t.chapters.len() => Some((chapter + 1) << 32),
        None => None,
    }
    .map(encode_cursor);
    let end = offset + piece.chars().count();
    let header = format!(
        "[{} — chapter {} \"{}\", characters {}-{} of {}{}]\n\n",
        b.meta.title,
        chapter,
        ch.title,
        offset,
        end,
        total,
        if next.is_some() {
            ", more follows"
        } else {
            ", end of book"
        }
    );
    let mut v = json!({
        "book_id": b.id, "chapter": chapter, "chapter_title": ch.title,
        "offset": offset, "end_offset": end, "chapter_chars": total,
        "chapters_total": t.chapters.len(), "text": piece, "note": COPYRIGHT_NOTE
    });
    if let Some(n) = next {
        v["next_cursor"] = json!(n);
    }
    let text = format!("{header}{}", v["text"].as_str().unwrap_or(""));
    Ok(ToolOutput {
        structured: v,
        text: Some(text),
    })
}

fn search_in_book(app: &Arc<App>, a: &Value, ctx: &Ctx) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let b = app.catalog.book(arg_id(a)?)?;
    let query = arg_str(a, "query").unwrap_or("");
    ctx.progress(0.0, Some(2.0), "reading the book");
    let t = app.texts.get(app.catalog.root(), &b)?;
    ctx.progress(1.0, Some(2.0), "searching");
    let opts = bookx_lookup::search::SearchOptions {
        whole_word: arg_bool(a, "whole_word"),
        case_sensitive: arg_bool(a, "case_sensitive"),
        regex: arg_bool(a, "regex"),
        max_results: (offset + limit + 1).min(500),
        context_chars: 80,
    };
    let texts: Vec<&str> = t.chapters.iter().map(|c| c.text.as_str()).collect();
    let r = bookx_lookup::search::search(&texts, query, &opts, Some(ctx.cancel))
        .map_err(|e| ToolError(format!("{e}")))?;
    let (items, next) = page(&r.hits, offset, limit);
    let hits: Vec<Value> = items
        .iter()
        .map(|h| {
            json!({"chapter": h.chapter, "chapter_title": t.chapters[h.chapter].title,
                "offset": t.chapters[h.chapter].text[..h.offset].chars().count(),
                "snippet": h.snippet})
        })
        .collect();
    Ok(ToolOutput::json(with_next(
        json!({"book_id": b.id, "hits": hits, "truncated": r.truncated, "cancelled": r.cancelled}),
        next,
    )))
}

fn mark_to_bold(snippet: &str) -> String {
    snippet.replace("<mark>", "**").replace("</mark>", "**")
}

fn search_all(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let query = arg_str(a, "query").unwrap_or("");
    let store = app.catalog.store()?;
    let books = app.catalog.books()?;
    let indexed = store.indexed_text_count();
    let raw = store.search_text(query, (offset + limit + 1).min(300) + 50)?;
    let hits: Vec<_> = raw
        .into_iter()
        .filter_map(|h| {
            let b = books.iter().find(|b| b.id == h.id)?;
            b.drm.is_empty().then_some((b, h))
        })
        .collect();
    let (items, next) = page(&hits, offset, limit);
    let mut v = json!({"indexed_books": indexed, "total_books": books.len(),
        "hits": items.iter().map(|(b, h)| json!({"book": summary(b), "snippet": mark_to_bold(&h.snippet)})).collect::<Vec<_>>()});
    if indexed < books.len() {
        v["note"] = json!(format!(
            "only {indexed} of {} books have their text indexed",
            books.len()
        ));
    }
    Ok(ToolOutput::json(with_next(v, next)))
}

fn get_annotations(app: &Arc<App>, a: &Value, ctx: &Ctx) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let books = app.catalog.books()?;
    let only = a.get("book_id").and_then(Value::as_u64);
    let kind = arg_str(a, "kind").unwrap_or("all");
    let color = arg_str(a, "color");
    let needle = arg_str(a, "query").unwrap_or("").to_lowercase();
    let mut rows: Vec<(chrono::DateTime<chrono::Utc>, Value)> = Vec::new();
    for (n, b) in books.iter().enumerate() {
        if ctx.should_stop() {
            break;
        }
        if n % 500 == 0 {
            ctx.progress(n as f64, Some(books.len() as f64), "scanning");
        }
        if only.is_some_and(|i| i != b.id) {
            continue;
        }
        if matches!(kind, "all" | "highlight" | "note") {
            for h in &b.highlights {
                if (kind == "note" && h.note.is_empty())
                    || color.is_some_and(|c| c != h.color)
                    || (!needle.is_empty()
                        && !h.text.to_lowercase().contains(&needle)
                        && !h.note.to_lowercase().contains(&needle))
                {
                    continue;
                }
                rows.push((
                    h.created,
                    json!({
                    "type": if h.note.is_empty() { "highlight" } else { "note" },
                    "book_id": b.id, "book_title": b.meta.title, "id": h.id, "chapter": h.spine,
                    "text": cut(&h.text, 1000), "note": cut(&h.note, 2000), "color": h.color,
                    "created": h.created.to_rfc3339()}),
                ));
            }
        }
        if matches!(kind, "all" | "bookmark") && color.is_none() {
            for m in &b.bookmarks {
                if !needle.is_empty() && !m.label.to_lowercase().contains(&needle) {
                    continue;
                }
                rows.push((
                    m.created,
                    json!({
                    "type": "bookmark", "book_id": b.id, "book_title": b.meta.title, "id": m.id,
                    "chapter": m.spine, "position": m.frac, "text": cut(&m.label, 500),
                    "created": m.created.to_rfc3339()}),
                ));
            }
        }
    }
    rows.sort_by(|x, y| y.0.cmp(&x.0));
    let values: Vec<Value> = rows.into_iter().map(|(_, v)| v).collect();
    let (items, next) = page(&values, offset, limit);
    Ok(ToolOutput::json(with_next(
        json!({"total": values.len(), "annotations": items}),
        next,
    )))
}

fn export_annotations(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    use bookx_core::annotations::AnnotationFormat as F;
    let b = app.catalog.book(arg_id(a)?)?;
    let fmt = match arg_str(a, "format").unwrap_or("markdown") {
        "json" => F::Json,
        "text" => F::Text,
        _ => F::Markdown,
    };
    let titles: Vec<String> = app
        .texts
        .get(app.catalog.root(), &b)
        .map(|t| t.chapters.iter().map(|c| c.title.clone()).collect())
        .unwrap_or_default();
    let content = bookx_core::annotations::export_annotations(&b, &titles, fmt);
    Ok(ToolOutput {
        structured: json!({"format": fmt.extension(), "content": content, "book_id": b.id}),
        text: Some(content),
    })
}

fn reading_stats(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    use chrono::{Duration, Utc};
    let days = a.get("days").and_then(Value::as_i64).unwrap_or(30);
    let books = app.catalog.books()?;
    let count = |s: &str| books.iter().filter(|b| b.status() == s).count();
    let library = json!({"books": books.len(), "finished": count("finished"),
        "reading": count("reading"), "unread": count("unread"),
        "highlights": books.iter().map(|b| b.highlights.len()).sum::<usize>()});
    let mut v = json!({"library": library, "window_days": days});
    if let Some(p) = &app.cfg.stats_path {
        if let Ok(store) = bookx_lookup::stats::StatsStore::load(p) {
            let since = Utc::now() - Duration::days(days);
            let recent: Vec<_> = store.sessions().iter().filter(|s| s.end >= since).collect();
            let secs: u64 = recent.iter().map(|s| s.duration_secs().max(0) as u64).sum();
            let words: u64 = recent.iter().map(|s| s.words).sum();
            let pages: u64 = recent.iter().map(|s| u64::from(s.pages)).sum();
            let owned: Vec<_> = recent.iter().map(|s| (*s).clone()).collect();
            let mut per: Vec<(String, bookx_lookup::stats::Totals)> =
                bookx_lookup::stats::book_totals(&owned)
                    .into_iter()
                    .collect();
            per.sort_by(|x, y| y.1.seconds.cmp(&x.1.seconds));
            let daily = store.daily(&Utc);
            let streak = bookx_lookup::stats::streaks(&daily, Utc::now().date_naive(), 60);
            v["sessions"] = json!({
                "count": recent.len(), "seconds": secs, "words": words, "pages": pages,
                "wpm": bookx_lookup::stats::reading_speed_wpm(&owned),
                "finished_books_total": store.finished().len(),
                "current_streak_days": streak.current, "longest_streak_days": streak.longest,
                "per_book": per.iter().take(10).map(|(id, t)| json!({
                    "book_id": id,
                    "title": id.parse::<u64>().ok().and_then(|i| books.iter().find(|b| b.id == i)).map(|b| b.meta.title.clone()),
                    "seconds": t.seconds, "words": t.words})).collect::<Vec<_>>()
            });
        }
    }
    if v["sessions"]["count"].as_u64().unwrap_or(0) == 0 {
        v["note"] = json!(
            "no reading sessions are recorded for this period; only library counts are reliable"
        );
    }
    Ok(ToolOutput::json(v))
}

fn recent_books(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .clamp(1, 100) as usize;
    let books = app.catalog.books()?;
    let added = arg_str(a, "kind") == Some("added");
    let mut v: Vec<&Book> = books
        .iter()
        .filter(|b| added || b.last_read.is_some())
        .collect();
    if added {
        v.sort_by(|x, y| y.added.cmp(&x.added));
    } else {
        v.sort_by(|x, y| y.last_read.cmp(&x.last_read));
    }
    Ok(ToolOutput::json(
        json!({"books": v.iter().take(limit).map(|b| {
        let mut s = summary(b);
        s["last_read"] = json!(b.last_read.map(|t| t.to_rfc3339()));
        s["added"] = json!(b.added.to_rfc3339());
        s
    }).collect::<Vec<_>>()}),
    ))
}

fn find_duplicates(app: &Arc<App>, a: &Value, ctx: &Ctx) -> Result<ToolOutput, ToolError> {
    let (offset, limit) = page_args(a)?;
    let threshold = a.get("threshold").and_then(Value::as_f64).unwrap_or(0.9);
    let books = app.catalog.books()?;
    ctx.progress(0.0, Some(1.0), "comparing books");
    let mut sets: Vec<Vec<u64>> = bookx_sources::dedup::find_duplicates(&books, threshold)
        .into_iter()
        .filter(|s| s.ids.len() > 1)
        .map(|s| s.ids)
        .collect();
    sets.sort();
    let (items, next) = page(&sets, offset, limit);
    let groups: Vec<Value> = items
        .iter()
        .map(|ids| {
            json!({"books": ids.iter().filter_map(|i| books.iter().find(|b| b.id == *i)).map(|b| {
                let mut s = summary(b);
                s["size"] = json!(b.size);
                s["highlights"] = json!(b.highlights.len());
                s
            }).collect::<Vec<_>>()})
        })
        .collect();
    Ok(ToolOutput::json(with_next(
        json!({"total": sets.len(), "groups": groups}),
        next,
    )))
}

/// Score and reasons of `other` being like `base`.
pub fn similarity(base: &Book, other: &Book) -> (f64, Vec<String>) {
    let mut score = 0.0;
    let mut why = Vec::new();
    if !base.meta.series.is_empty() && base.meta.series.eq_ignore_ascii_case(&other.meta.series) {
        score += 5.0;
        why.push(format!("same series: {}", other.meta.series));
    }
    let shared_authors: Vec<&String> = base
        .meta
        .authors
        .iter()
        .filter(|a| other.meta.authors.iter().any(|o| o.eq_ignore_ascii_case(a)))
        .collect();
    if !shared_authors.is_empty() {
        score += 4.0;
        why.push(format!(
            "same author: {}",
            shared_authors
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let tags: Vec<&String> = base
        .meta
        .tags
        .iter()
        .filter(|t| other.meta.tags.iter().any(|o| o.eq_ignore_ascii_case(t)))
        .collect();
    if !tags.is_empty() {
        let union = base.meta.tags.len() + other.meta.tags.len() - tags.len();
        score += 3.0 * tags.len() as f64 / union.max(1) as f64 + 0.5;
        why.push(format!(
            "shared tags: {}",
            tags.iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !base.meta.language.is_empty() && base.meta.language == other.meta.language && score > 0.0 {
        score += 0.5;
    }
    if !base.meta.publisher.is_empty() && base.meta.publisher == other.meta.publisher && score > 0.0
    {
        score += 0.5;
        why.push(format!("same publisher: {}", other.meta.publisher));
    }
    (score, why)
}

fn similar_books(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let limit = a
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(10)
        .clamp(1, 50) as usize;
    let base = app.catalog.book(arg_id(a)?)?;
    let books = app.catalog.books()?;
    let mut scored: Vec<(f64, Vec<String>, &Book)> = books
        .iter()
        .filter(|b| b.id != base.id)
        .filter_map(|b| {
            let (s, why) = similarity(&base, b);
            (s > 0.0).then_some((s, why, b))
        })
        .collect();
    scored.sort_by(|x, y| {
        y.0.partial_cmp(&x.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(x.2.id.cmp(&y.2.id))
    });
    Ok(ToolOutput::json(
        json!({"books": scored.iter().take(limit).map(|(s, why, b)| {
        let mut v = summary(b);
        v["score"] = json!((s * 100.0).round() / 100.0);
        v["reasons"] = json!(why);
        v
    }).collect::<Vec<_>>()}),
    ))
}

fn check_book(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    use bookx_core::check::Severity;
    let b = app.catalog.book(arg_id(a)?)?;
    let path = app
        .catalog
        .root()
        .join(b.file_for(bookx_core::formats::Target::Reader));
    let archive = bookx_core::archive::Archive::open(&path)
        .map_err(|_| ToolError("the book has no EPUB version to check".into()))?;
    let issues = bookx_core::check::check_book(&archive);
    let n = |s: Severity| issues.iter().filter(|i| i.severity == s).count();
    Ok(ToolOutput::json(json!({
        "book_id": b.id, "errors": n(Severity::Error), "warnings": n(Severity::Warning),
        "infos": n(Severity::Info),
        "issues": issues.iter().take(30).map(|i| json!({
            "severity": match i.severity { Severity::Error => "error", Severity::Warning => "warning", Severity::Info => "info" },
            "file": i.file, "line": i.line, "message": cut(&i.message, 300)})).collect::<Vec<_>>(),
        "more": issues.len().saturating_sub(30)
    })))
}

fn convert_preview(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let b = app.catalog.book(arg_id(a)?)?;
    let ext = arg_str(a, "format")
        .unwrap_or("")
        .trim_start_matches('.')
        .to_lowercase();
    if !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return fail("format must be a file extension such as mobi or fb2");
    }
    let path = app
        .catalog
        .root()
        .join(b.file_for(bookx_core::formats::Target::Reader));
    let book = bookx_core::book::OpenBook::open(&path)
        .map_err(|_| ToolError("the book has no EPUB version to inspect".into()))?;
    let r = bookx_convert::report::build_report(&book, &ext, &[]);
    Ok(ToolOutput::json(json!({
        "book_id": b.id, "format": ext, "lost": r.lost, "words": r.words, "images": r.images,
        "toc_entries": r.toc_entries, "broken_links": r.broken_links,
        "note": if r.lost.is_empty() { "nothing would be lost" } else { "these are not carried over" }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_core::opf::Metadata;
    use chrono::Utc;

    fn book(id: u64, title: &str, author: &str, series: &str, tags: &[&str]) -> Book {
        Book {
            id,
            file: format!("{id}.epub"),
            cover: None,
            meta: Metadata {
                title: title.into(),
                authors: vec![author.into()],
                series: series.into(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                ..Default::default()
            },
            added: Utc::now(),
            modified: Utc::now(),
            last_read: None,
            size: 1,
            progress: 0.0,
            position: None,
            bookmarks: vec![],
            highlights: vec![],
            formats: vec![],
            resources: vec![],
            drm: String::new(),
        }
    }

    #[test]
    fn similarity_weighs_series_author_and_tags() {
        let a = book(1, "A", "Ann", "Saga", &["sf", "space"]);
        let same_series = book(2, "B", "Bob", "saga", &[]);
        let same_author = book(3, "C", "ann", "", &[]);
        let tags = book(4, "D", "Zed", "", &["sf"]);
        let none = book(5, "E", "Zed", "", &["cooking"]);
        let s = |b| similarity(&a, b).0;
        assert!(s(&same_series) > s(&same_author));
        assert!(s(&same_author) > s(&tags));
        assert!(s(&tags) > 0.0);
        assert_eq!(s(&none), 0.0);
        assert!(similarity(&a, &same_series).1[0].contains("same series"));
    }

    #[test]
    fn pages_and_cursors() {
        let items: Vec<u32> = (0..5).collect();
        let (p, next) = page(&items, 0, 2);
        assert_eq!(p, vec![0, 1]);
        let off = decode_cursor(&next.unwrap()).unwrap();
        let (p, next) = page(&items, off, 2);
        assert_eq!(p, vec![2, 3]);
        let (p, next2) = page(&items, decode_cursor(&next.unwrap()).unwrap(), 2);
        assert_eq!(p, vec![4]);
        assert!(next2.is_none());
        assert!(decode_cursor("!!").is_err());
    }

    #[test]
    fn every_published_schema_is_valid() {
        let mut cfg = Config::new(std::path::Path::new("/nonexistent"));
        cfg.write = crate::app::WritePolicy::Locked;
        cfg.allow_network = true;
        cfg.fetcher = Some(Arc::new(bookx_sources::fetch::MockFetcher::new()));
        let all = defs(&cfg);
        assert!(all.len() >= 27, "{}", all.len());
        let mut names = std::collections::HashSet::new();
        for d in &all {
            assert!(names.insert(d.name), "duplicate {}", d.name);
            schema::schema_is_valid(&d.input).unwrap_or_else(|e| panic!("{}: {e}", d.name));
            schema::schema_is_valid(&d.output).unwrap_or_else(|e| panic!("{}: {e}", d.name));
            assert_eq!(d.input["type"], "object");
            assert!(!d.description.is_empty() && !d.title.is_empty());
            let j = d.to_json();
            assert_eq!(j["annotations"]["readOnlyHint"], !d.write);
        }
        // read-only configuration: no write tool is listed
        let ro = defs(&Config::new(std::path::Path::new("/x")));
        assert!(ro.iter().all(|d| !d.write));
        assert!(ro.len() < all.len());
    }
}
