//! MCP resources: `bookx://book/{id}`, `bookx://book/{id}/cover`, `bookx://book/{id}/chapter/{n}`
//! and `bookx://library/stats`.

use std::sync::Arc;

use base64::Engine;
use serde_json::{Value, json};

use crate::app::App;
use crate::server::{INTERNAL_ERROR, RESOURCE_NOT_FOUND, RpcError};
use crate::tools;

pub fn templates() -> Vec<Value> {
    vec![
        json!({"uriTemplate":"bookx://book/{id}","name":"book","title":"Book metadata",
            "description":"Metadata of one book as JSON.","mimeType":"application/json"}),
        json!({"uriTemplate":"bookx://book/{id}/cover","name":"book-cover","title":"Book cover",
            "description":"The cover image.","mimeType":"image/jpeg"}),
        json!({"uriTemplate":"bookx://book/{id}/chapter/{n}","name":"book-chapter","title":"Chapter text",
            "description":"Plain text of chapter n (0-based, see get_toc), cut at the server's size limit.","mimeType":"text/plain"}),
    ]
}

pub fn list(app: &Arc<App>) -> Result<Vec<Value>, RpcError> {
    let books = app
        .catalog
        .books()
        .map_err(|e| RpcError::new(INTERNAL_ERROR, app.scrub(&format!("{e:#}"))))?;
    let mut v = vec![json!({"uri":"bookx://library/stats","name":"library-stats",
        "title":"Library statistics","description":"Counts of books by status.","mimeType":"application/json"})];
    v.extend(books.iter().map(|b| {
        json!({"uri": format!("bookx://book/{}", b.id), "name": format!("book-{}", b.id),
            "title": b.meta.title, "description": b.meta.authors_display(), "mimeType":"application/json"})
    }));
    Ok(v)
}

#[derive(Debug, PartialEq)]
pub enum Target {
    Stats,
    Book(u64),
    Cover(u64),
    Chapter(u64, usize),
}

pub fn parse(uri: &str) -> Option<Target> {
    let rest = uri.strip_prefix("bookx://")?;
    let parts: Vec<&str> = rest.split('/').collect();
    match parts.as_slice() {
        ["library", "stats"] => Some(Target::Stats),
        ["book", id] => Some(Target::Book(id.parse().ok()?)),
        ["book", id, "cover"] => Some(Target::Cover(id.parse().ok()?)),
        ["book", id, "chapter", n] => Some(Target::Chapter(id.parse().ok()?, n.parse().ok()?)),
        _ => None,
    }
}

fn not_found(msg: impl Into<String>) -> RpcError {
    RpcError::new(RESOURCE_NOT_FOUND, msg)
}

pub fn read(app: &Arc<App>, uri: &str) -> Result<Vec<Value>, RpcError> {
    let target = parse(uri).ok_or_else(|| not_found(format!("unknown resource: {uri}")))?;
    let book = |id: u64| app.catalog.book(id).map_err(|e| not_found(e.0));
    match target {
        Target::Stats => {
            let books = app
                .catalog
                .books()
                .map_err(|e| RpcError::new(INTERNAL_ERROR, app.scrub(&format!("{e:#}"))))?;
            let n = |s: &str| books.iter().filter(|b| b.status() == s).count();
            let v = json!({"books": books.len(), "finished": n("finished"), "reading": n("reading"),
                "unread": n("unread"),
                "authors": books.iter().flat_map(|b| b.meta.authors.iter()).collect::<std::collections::HashSet<_>>().len(),
                "highlights": books.iter().map(|b| b.highlights.len()).sum::<usize>()});
            Ok(vec![
                json!({"uri":uri,"mimeType":"application/json","text":v.to_string()}),
            ])
        }
        Target::Book(id) => {
            let b = book(id)?;
            let out = tools::book_json(app, &b).map_err(|e| not_found(app.scrub(&e.0)))?;
            Ok(vec![
                json!({"uri":uri,"mimeType":"application/json","text":out.to_string()}),
            ])
        }
        Target::Cover(id) => {
            let b = book(id)?;
            let rel = b
                .cover
                .clone()
                .ok_or_else(|| not_found("this book has no cover"))?;
            let path = app.catalog.root().join(&rel);
            let real = path
                .canonicalize()
                .map_err(|_| not_found("the cover file is missing"))?;
            let root = app
                .catalog
                .root()
                .canonicalize()
                .map_err(|_| not_found("the library folder is missing"))?;
            if !real.starts_with(&root) {
                return Err(not_found("the cover is outside the library"));
            }
            let bytes = std::fs::read(&real).map_err(|_| not_found("the cover cannot be read"))?;
            if bytes.len() > 8 << 20 {
                return Err(not_found("the cover is too large"));
            }
            let ext = real.extension().and_then(|e| e.to_str()).unwrap_or("");
            let mime = bookx_serve::http::mime_for_extension(ext);
            Ok(vec![json!({"uri":uri,"mimeType":mime,
                "blob": base64::engine::general_purpose::STANDARD.encode(bytes)})])
        }
        Target::Chapter(id, n) => {
            let b = book(id)?;
            let t = app
                .texts
                .get(app.catalog.root(), &b)
                .map_err(|e| not_found(app.scrub(&e.0)))?;
            let ch = t
                .chapters
                .get(n)
                .ok_or_else(|| not_found(format!("the book has {} chapters", t.chapters.len())))?;
            let (text, next) = crate::text::slice_chars(&ch.text, 0, app.cfg.max_text_chars);
            let mut text = text;
            if next.is_some() {
                text.push_str("\n\n[… cut at the size limit; use read_book_text to continue]");
            }
            Ok(vec![json!({"uri":uri,"mimeType":"text/plain","text":text})])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uris_parse() {
        assert_eq!(parse("bookx://library/stats"), Some(Target::Stats));
        assert_eq!(parse("bookx://book/7"), Some(Target::Book(7)));
        assert_eq!(parse("bookx://book/7/cover"), Some(Target::Cover(7)));
        assert_eq!(
            parse("bookx://book/7/chapter/3"),
            Some(Target::Chapter(7, 3))
        );
        for bad in [
            "bookx://book/x",
            "bookx://book/7/chapter",
            "http://book/7",
            "bookx://",
            "bookx://book/7/chapter/-1",
            "bookx://book/7/../x",
        ] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }
}
