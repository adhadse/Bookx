//! MCP prompts: ready-made starting points that use the tools and resources.

use std::sync::Arc;

use serde_json::{json, Value};

use crate::app::App;
use crate::resources;
use crate::server::RpcError;

fn arg(name: &str, description: &str, required: bool) -> Value {
    json!({"name":name,"description":description,"required":required})
}

pub fn list() -> Vec<Value> {
    vec![
        json!({"name":"summarise-chapter","title":"Summarise a chapter",
            "description":"Summarise one chapter of a book.",
            "arguments":[arg("book_id","Id of the book",true),arg("chapter","0-based chapter index (default 0)",false)]}),
        json!({"name":"reading-recommendations","title":"Reading recommendations",
            "description":"Suggest what to read next from the library, based on what was read and liked.",
            "arguments":[arg("mood","What the user feels like reading",false)]}),
        json!({"name":"study-notes-from-highlights","title":"Study notes from highlights",
            "description":"Turn the highlights and notes of a book into structured study notes.",
            "arguments":[arg("book_id","Id of the book",true)]}),
        json!({"name":"find-quotes-about","title":"Find quotes about a topic",
            "description":"Find passages about a topic in one book or in the whole library.",
            "arguments":[arg("topic","The topic or phrase",true),arg("book_id","Limit to this book",false)]}),
    ]
}

fn user(text: String) -> Value {
    json!({"role":"user","content":{"type":"text","text":text}})
}

fn required<'a>(args: &'a Value, k: &str) -> Result<&'a str, RpcError> {
    args.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| RpcError::invalid(format!("missing argument `{k}`")))
}

fn book_id(args: &Value) -> Result<u64, RpcError> {
    required(args, "book_id")?
        .trim()
        .parse()
        .map_err(|_| RpcError::invalid("`book_id` must be a number"))
}

pub fn get(app: &Arc<App>, name: &str, args: &Value) -> Result<Value, RpcError> {
    match name {
        "summarise-chapter" => {
            let id = book_id(args)?;
            let ch: usize = args
                .get("chapter")
                .and_then(Value::as_str)
                .and_then(|c| c.trim().parse().ok())
                .unwrap_or(0);
            let uri = format!("bookx://book/{id}/chapter/{ch}");
            let content = resources::read(app, &uri)?;
            Ok(json!({"description":"Summarise a chapter","messages":[
                user("Summarise this chapter in a short paragraph followed by 3-5 key points. Quote at most a sentence or two.".into()),
                {"role":"user","content":{"type":"resource","resource":content[0].clone()}}
            ]}))
        }
        "reading-recommendations" => {
            let mood = args.get("mood").and_then(Value::as_str).unwrap_or("");
            Ok(
                json!({"description":"Recommend books","messages":[user(format!(
                    "Recommend what I should read next from my Bookx library{}. Use recent_books and get_reading_stats to see what I read, get_annotations for what I highlighted, search_books (for example with status:new rating:>=4 or tag:) for candidates and similar_books to compare. Suggest 3-5 unread books with one sentence of reasoning each.",
                    if mood.is_empty() { String::new() } else { format!(" — I feel like: {mood}") }
                ))]}),
            )
        }
        "study-notes-from-highlights" => {
            let id = book_id(args)?;
            let book = app.catalog.book(id).map_err(|e| RpcError::invalid(e.0))?;
            let titles: Vec<String> = app
                .texts
                .get(app.catalog.root(), &book)
                .map(|t| t.chapters.iter().map(|c| c.title.clone()).collect())
                .unwrap_or_default();
            let md = bookx_core::annotations::export_annotations(
                &book,
                &titles,
                bookx_core::annotations::AnnotationFormat::Markdown,
            );
            Ok(json!({"description":"Study notes","messages":[user(format!(
                "Turn my highlights and notes of this book into study notes: group them by theme, add a one-line takeaway per theme and list open questions. Keep my own notes verbatim.\n\n{md}"
            ))]}))
        }
        "find-quotes-about" => {
            let topic = required(args, "topic")?;
            let scope = match args.get("book_id").and_then(Value::as_str) {
                Some(id) if !id.trim().is_empty() => format!("in the book with id {} (use search_in_book)", id.trim()),
                _ => "across my library (use search_all_books_text, then search_in_book on the best books)".into(),
            };
            Ok(json!({"description":"Find quotes","messages":[user(format!(
                "Find passages about \"{topic}\" {scope}. For each, give the book, chapter and a short quote of at most two sentences."
            ))]}))
        }
        other => Err(RpcError::invalid(format!("unknown prompt: {other}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_declare_their_arguments() {
        let l = list();
        assert!(l.len() >= 4);
        for p in &l {
            assert!(p["name"].is_string() && p["description"].is_string());
            assert!(p["arguments"].is_array());
        }
    }
}
