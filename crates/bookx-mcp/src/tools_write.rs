//! Tools that change the library. They are offered only when writing is allowed, and every change
//! goes through `Library` (so metadata edits and imports land in the operation log and can be
//! undone in the app) and is written to the audit log by the call path.

use std::path::Path;
use std::sync::Arc;

use bookx_core::library::{AddOutcome, Bookmark, Highlight};
use bookx_core::opf::Metadata;
use bookx_store::ReadingStatus;
use chrono::Utc;
use serde_json::{json, Value};

use crate::app::{fail, App, Config, ToolError, ToolOutput};
use crate::server::Ctx;
use crate::text::BookText;
use crate::tools::{arg_id, arg_str, summary, write_def, ToolDef};

const COLORS: [&str; 5] = ["yellow", "purple", "blue", "red", "green"];

fn book_id() -> Value {
    json!({"type":"integer","minimum":1,"description":"Id of the book (from search_books)."})
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type":"object","properties":props,"required":required,"additionalProperties":false})
}

pub fn defs(cfg: &Config) -> Vec<ToolDef> {
    let tag_list = json!({"type":"array","items":{"type":"string","minLength":1,"maxLength":100},"maxItems":50});
    let mut v = vec![
        write_def("add_highlight", "Add a highlight",
            "Highlights a passage of a book: give the exact text (and the chapter to look in; otherwise the first match in the book is used) and optionally a note. Shows up in the reader and can be removed there.",
            obj(json!({"book_id":book_id(),
                "text":{"type":"string","minLength":2,"maxLength":2000},
                "chapter":{"type":"integer","minimum":0},
                "note":{"type":"string","maxLength":4000},
                "color":{"type":"string","enum":COLORS}}), &["book_id","text"]), false),
        write_def("add_note", "Add a note",
            "Adds a note to a book: attached to a passage when `text` is given (a highlight with a note), otherwise a bookmark labelled with the note at the start of the chapter.",
            obj(json!({"book_id":book_id(),"note":{"type":"string","minLength":1,"maxLength":4000},
                "text":{"type":"string","maxLength":2000},"chapter":{"type":"integer","minimum":0}}), &["book_id","note"]), false),
        write_def("set_rating", "Set the rating",
            "Sets the star rating of a book (0 clears it, half stars allowed). Written into the book file; undoable in the app.",
            obj(json!({"book_id":book_id(),"stars":{"type":"number","minimum":0,"maximum":5}}), &["book_id","stars"]), true),
        write_def("add_tags", "Add tags", "Adds tags to a book (undoable in the app).",
            obj(json!({"book_id":book_id(),"tags":tag_list.clone()}), &["book_id","tags"]), true),
        write_def("remove_tags", "Remove tags", "Removes tags from a book (undoable in the app).",
            obj(json!({"book_id":book_id(),"tags":tag_list}), &["book_id","tags"]), true),
        write_def("mark_read", "Mark read or unread",
            "Marks a book finished (progress 100%) or resets it to unread.",
            obj(json!({"book_id":book_id(),"read":{"type":"boolean","description":"Default true."}}), &["book_id"]), true),
        write_def("add_to_shelf", "Add to a shelf",
            "Puts a book on a shelf (collection). The shelf must exist unless create is true.",
            obj(json!({"book_id":book_id(),"shelf":{"type":"string","minLength":1,"maxLength":100},
                "create":{"type":"boolean"}}), &["book_id","shelf"]), true),
        write_def("update_metadata", "Update metadata",
            "Changes metadata fields of a book (only the fields you give). Written into the book file; undoable in the app.",
            obj(json!({"book_id":book_id(),"fields":{"type":"object","additionalProperties":false,"properties":{
                "title":{"type":"string","minLength":1,"maxLength":500},
                "authors":{"type":"array","items":{"type":"string","minLength":1,"maxLength":200},"maxItems":20},
                "series":{"type":"string","maxLength":200},
                "series_index":{"type":"number","minimum":0},
                "publisher":{"type":"string","maxLength":200},
                "language":{"type":"string","maxLength":20},
                "description":{"type":"string","maxLength":20000},
                "published":{"type":"string","maxLength":40},
                "tags":{"type":"array","items":{"type":"string","minLength":1,"maxLength":100},"maxItems":100}}}}), &["book_id","fields"]), true),
    ];
    if cfg.allow_network && cfg.fetcher.is_some() {
        let mut d = write_def("fetch_metadata", "Fetch metadata online",
            "Looks the book up in Open Library, Google Books and Wikidata and shows what would change. With apply=true the missing fields are filled in (mode fill_empty) or all found fields replaced (mode replace); undoable in the app. Sends the title, author and ISBN to those services.",
            obj(json!({"book_id":book_id(),"apply":{"type":"boolean"},
                "mode":{"type":"string","enum":["fill_empty","replace"]}}), &["book_id"]), true);
        d.open_world = true;
        v.push(d);
    }
    if !cfg.allow_dirs.is_empty() {
        let mut d = write_def("add_book_from_path", "Add a book from a file",
            "Imports a book file into the library. The file must be inside one of the folders the user allowed for this server.",
            obj(json!({"path":{"type":"string","minLength":1,"maxLength":2000},
                "allow_duplicate":{"type":"boolean"}}), &["path"]), false);
        d.description.push_str(&format!(
            " Allowed folders: {}.",
            cfg.allow_dirs
                .iter()
                .map(|p| p.display().to_string())
                .collect::<Vec<_>>()
                .join(", ")
        ));
        v.push(d);
    }
    v
}

pub fn call(app: &Arc<App>, name: &str, a: &Value, ctx: &Ctx) -> Result<ToolOutput, ToolError> {
    match name {
        "add_highlight" => add_highlight(app, a),
        "add_note" => add_note(app, a),
        "set_rating" => {
            let stars = a.get("stars").and_then(Value::as_f64).unwrap_or(0.0);
            let r = (stars * 2.0).round().clamp(0.0, 10.0) as u8;
            edit_meta(
                app,
                arg_id(a)?,
                &format!("rating set to {} stars", f64::from(r) / 2.0),
                |m| m.rating = r,
            )
        }
        "add_tags" => {
            let tags = tag_args(a);
            edit_meta(app, arg_id(a)?, "tags added", move |m| {
                for t in tags {
                    if !m.tags.iter().any(|x| x.eq_ignore_ascii_case(&t)) {
                        m.tags.push(t);
                    }
                }
            })
        }
        "remove_tags" => {
            let tags = tag_args(a);
            edit_meta(app, arg_id(a)?, "tags removed", move |m| {
                m.tags
                    .retain(|x| !tags.iter().any(|t| t.eq_ignore_ascii_case(x)));
            })
        }
        "mark_read" => mark_read(app, a),
        "add_to_shelf" => add_to_shelf(app, a),
        "update_metadata" => update_metadata(app, a),
        "fetch_metadata" => fetch_metadata(app, a, ctx),
        "add_book_from_path" => add_book_from_path(app, a),
        other => fail(format!("unknown tool {other}")),
    }
}

fn tag_args(a: &Value) -> Vec<String> {
    a.get("tags")
        .and_then(Value::as_array)
        .map(|t| {
            t.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn done(message: String, book: Option<Value>) -> Result<ToolOutput, ToolError> {
    let mut v = json!({"ok": true, "message": message});
    if let Some(b) = book {
        v["book"] = b;
    }
    Ok(ToolOutput::json(v))
}

/// Edits the metadata of one book through the library, so it is logged for undo. Does nothing
/// (and says so) when the edit changes nothing.
fn edit_meta(
    app: &Arc<App>,
    id: u64,
    what: &str,
    f: impl FnOnce(&mut Metadata),
) -> Result<ToolOutput, ToolError> {
    let (changed, book) = app.with_library(|lib, _| {
        let before = lib
            .get(id)
            .ok_or_else(|| ToolError(format!("there is no book with id {id}")))?
            .meta
            .clone();
        let mut m = before.clone();
        f(&mut m);
        if m == before {
            return Ok((false, summary(lib.get(id).expect("checked"))));
        }
        lib.update_metadata(id, m)?;
        Ok((true, summary(lib.get(id).expect("checked"))))
    })?;
    done(
        if changed {
            what.to_string()
        } else {
            "nothing to change".into()
        },
        Some(book),
    )
}

/// Where `needle` (whitespace collapsed) is in the chapters: `(chapter, start, end, exact text)`
/// with character offsets in the reader's numbering.
pub fn locate(
    t: &BookText,
    chapter: Option<usize>,
    needle: &str,
) -> Option<(usize, usize, usize, String)> {
    let needle = needle.split_whitespace().collect::<Vec<_>>().join(" ");
    if needle.is_empty() {
        return None;
    }
    let range: Vec<usize> = match chapter {
        Some(c) if c < t.chapters.len() => vec![c],
        Some(_) => return None,
        None => (0..t.chapters.len()).collect(),
    };
    for exact in [true, false] {
        for &c in &range {
            let text = &t.chapters[c].text;
            let found = if exact {
                text.find(&needle)
            } else {
                text.to_ascii_lowercase().find(&needle.to_ascii_lowercase())
            };
            if let Some(b) = found {
                let start = text[..b].chars().count();
                let len = needle.chars().count();
                let real: String = text.chars().skip(start).take(len).collect();
                return Some((c, start, start + len, real));
            }
        }
    }
    None
}

fn add_highlight(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let id = arg_id(a)?;
    let b = app.catalog.book(id)?;
    let t = app.texts.get(app.catalog.root(), &b)?;
    let text = arg_str(a, "text").unwrap_or("");
    let chapter = a.get("chapter").and_then(Value::as_u64).map(|c| c as usize);
    let Some((c, start, end, exact)) = locate(&t, chapter, text) else {
        return fail("that text was not found in the book (copy it exactly from read_book_text)");
    };
    push_highlight(
        app,
        id,
        c,
        start,
        end,
        exact,
        arg_str(a, "note").unwrap_or(""),
        arg_str(a, "color").unwrap_or("yellow"),
    )
}

#[allow(clippy::too_many_arguments)]
fn push_highlight(
    app: &Arc<App>,
    id: u64,
    chapter: usize,
    start: usize,
    end: usize,
    text: String,
    note: &str,
    color: &str,
) -> Result<ToolOutput, ToolError> {
    let hid = uuid::Uuid::new_v4().to_string();
    let book = app.with_library(|lib, _| {
        let b = lib
            .get_mut(id)
            .ok_or_else(|| ToolError(format!("there is no book with id {id}")))?;
        b.highlights.push(Highlight {
            id: hid.clone(),
            spine: chapter,
            start: start as i32,
            end: end as i32,
            color: color.to_string(),
            style: String::new(),
            note: note.trim().to_string(),
            text,
            created: Utc::now(),
        });
        Ok(summary(b))
    })?;
    let mut out = ToolOutput::json(json!({"ok": true, "message": "highlight added",
        "highlight_id": hid, "chapter": chapter, "book": book}));
    out.text = None;
    Ok(out)
}

fn add_note(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let id = arg_id(a)?;
    let note = arg_str(a, "note").unwrap_or("").trim().to_string();
    let chapter = a.get("chapter").and_then(Value::as_u64).map(|c| c as usize);
    if let Some(text) = arg_str(a, "text").filter(|t| !t.trim().is_empty()) {
        let b = app.catalog.book(id)?;
        let t = app.texts.get(app.catalog.root(), &b)?;
        let Some((c, start, end, exact)) = locate(&t, chapter, text) else {
            return fail(
                "that text was not found in the book (copy it exactly from read_book_text)",
            );
        };
        return push_highlight(app, id, c, start, end, exact, &note, "yellow");
    }
    let spine = chapter.unwrap_or(0);
    let book = app.with_library(|lib, _| {
        let b = lib
            .get_mut(id)
            .ok_or_else(|| ToolError(format!("there is no book with id {id}")))?;
        b.bookmarks.push(Bookmark {
            id: uuid::Uuid::new_v4().to_string(),
            spine,
            frac: 0.0,
            label: note.clone(),
            created: Utc::now(),
        });
        Ok(summary(b))
    })?;
    done("note added as a bookmark".into(), Some(book))
}

fn mark_read(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let id = arg_id(a)?;
    let read = a.get("read").and_then(Value::as_bool).unwrap_or(true);
    let book = app.with_library(|lib, store| {
        if lib.get(id).is_none() {
            return fail(format!("there is no book with id {id}"));
        }
        lib.mark_read(id, read);
        store.set_status(
            id,
            if read {
                ReadingStatus::Finished
            } else {
                ReadingStatus::New
            },
        )?;
        Ok(summary(lib.get(id).expect("checked")))
    })?;
    done(
        if read {
            "marked as read"
        } else {
            "marked as unread"
        }
        .into(),
        Some(book),
    )
}

fn add_to_shelf(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let id = arg_id(a)?;
    let shelf = arg_str(a, "shelf").unwrap_or("").trim().to_string();
    let create = a.get("create").and_then(Value::as_bool).unwrap_or(false);
    let message = app.with_library(|lib, store| {
        if lib.get(id).is_none() {
            return fail(format!("there is no book with id {id}"));
        }
        let existing = store
            .list_collections()?
            .into_iter()
            .find(|c| c.name.eq_ignore_ascii_case(&shelf));
        let (cid, name) = match existing {
            Some(c) => (c.id, c.name),
            None if create => (store.create_collection(&shelf)?, shelf.clone()),
            None => {
                return fail(format!(
                    "there is no shelf \"{shelf}\" (pass create: true to make it)"
                ))
            }
        };
        store.add_to_collection(cid, id)?;
        Ok(format!("added to the shelf \"{name}\""))
    })?;
    done(message, None)
}

fn update_metadata(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let fields = a.get("fields").cloned().unwrap_or_else(|| json!({}));
    if fields.as_object().is_none_or(|o| o.is_empty()) {
        return fail("`fields` must name at least one field to change");
    }
    let strings = |k: &str| -> Option<Vec<String>> {
        fields.get(k).and_then(Value::as_array).map(|v| {
            v.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
    };
    let text = |k: &str| {
        fields
            .get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().to_string())
    };
    let (title, authors, tags) = (text("title"), strings("authors"), strings("tags"));
    let (series, publisher, language, description, published) = (
        text("series"),
        text("publisher"),
        text("language"),
        text("description"),
        text("published"),
    );
    let index = fields.get("series_index").and_then(Value::as_f64);
    edit_meta(app, arg_id(a)?, "metadata updated", move |m| {
        if let Some(t) = title {
            if m.title_sort.is_empty() || m.title_sort == m.title {
                m.title_sort = t.clone();
            }
            m.title = t;
        }
        if let Some(a) = authors {
            m.author_sort = a.first().map(|f| sort_name(f)).unwrap_or_default();
            m.authors = a;
        }
        if let Some(v) = series {
            m.series = v;
        }
        if let Some(v) = index {
            m.series_index = v;
        }
        if let Some(v) = publisher {
            m.publisher = v;
        }
        if let Some(v) = language {
            m.language = v;
        }
        if let Some(v) = description {
            m.description = v;
        }
        if let Some(v) = published {
            m.published = v;
        }
        if let Some(v) = tags {
            m.tags = v;
        }
    })
}

/// "Ann Lee" becomes "Lee, Ann"; names already written that way stay.
pub fn sort_name(name: &str) -> String {
    if name.contains(',') {
        return name.to_string();
    }
    match name.rsplit_once(' ') {
        Some((first, last)) => format!("{last}, {first}"),
        None => name.to_string(),
    }
}

/// `current` with what `found` knows. `fill_empty` only fills empty fields.
pub fn merge_found(current: &Metadata, found: &Metadata, replace: bool) -> Metadata {
    let mut m = current.clone();
    let take = |cur: &mut String, new: &str| {
        if !new.trim().is_empty() && (replace || cur.trim().is_empty()) {
            *cur = new.trim().to_string();
        }
    };
    take(&mut m.publisher, &found.publisher);
    take(&mut m.language, &found.language);
    take(&mut m.description, &found.description);
    take(&mut m.published, &found.published);
    take(&mut m.series, &found.series);
    if (replace || m.series_index == 0.0) && !found.series.is_empty() {
        m.series_index = found.series_index;
    }
    if !found.authors.is_empty() && (replace || m.authors.is_empty()) {
        m.authors = found.authors.clone();
    }
    for t in &found.tags {
        if !m.tags.iter().any(|x| x.eq_ignore_ascii_case(t)) {
            m.tags.push(t.clone());
        }
    }
    for i in &found.identifiers {
        if !m
            .identifiers
            .iter()
            .any(|x| x.scheme.eq_ignore_ascii_case(&i.scheme) && x.value == i.value)
        {
            m.identifiers.push(i.clone());
        }
    }
    m
}

fn changed_fields(a: &Metadata, b: &Metadata) -> Vec<&'static str> {
    let mut v = Vec::new();
    let mut chk = |name, differs: bool| {
        if differs {
            v.push(name)
        }
    };
    chk("authors", a.authors != b.authors);
    chk("publisher", a.publisher != b.publisher);
    chk("language", a.language != b.language);
    chk("description", a.description != b.description);
    chk("published", a.published != b.published);
    chk(
        "series",
        a.series != b.series || a.series_index != b.series_index,
    );
    chk("tags", a.tags != b.tags);
    chk("identifiers", a.identifiers != b.identifiers);
    v
}

fn fetch_metadata(app: &Arc<App>, a: &Value, ctx: &Ctx) -> Result<ToolOutput, ToolError> {
    let fetcher = app
        .cfg
        .fetcher
        .clone()
        .filter(|_| app.cfg.allow_network)
        .ok_or_else(|| ToolError("network access is not allowed for this server".into()))?;
    let id = arg_id(a)?;
    let book = app.catalog.book(id)?;
    let isbn = book
        .meta
        .identifiers
        .iter()
        .filter(|i| i.scheme.eq_ignore_ascii_case("isbn"))
        .find_map(|i| bookx_sources::isbn::canonical(&i.value));
    let q = bookx_core::contracts::MetadataQuery {
        title: book.meta.title.trim().to_string(),
        authors: book.meta.authors.clone(),
        isbn,
    };
    if q.title.is_empty() && q.isbn.is_none() {
        return fail("the book has neither a title nor an ISBN to look up");
    }
    ctx.progress(0.0, Some(1.0), "asking the metadata sources");
    let res = bookx_sources::sources::SourceSet::standard(fetcher).search(&q)?;
    let Some(best) = res.best else {
        return done("no source knew this book well enough".into(), None);
    };
    let replace = arg_str(a, "mode") == Some("replace");
    let merged = merge_found(&book.meta, &best.meta, replace);
    let changed = changed_fields(&book.meta, &merged);
    let apply = a.get("apply").and_then(Value::as_bool).unwrap_or(false);
    let mut v = json!({"ok": true, "applied": false, "confidence": best.confidence,
        "sources": best.sources, "would_change": changed,
        "found": {"title": best.meta.title, "authors": best.meta.authors, "publisher": best.meta.publisher,
            "published": best.meta.published, "series": best.meta.series, "tags": best.meta.tags},
        "message": format!("{} field(s) would change", changed.len())});
    if apply && !changed.is_empty() {
        let target = merged.clone();
        app.with_library(|lib, store| {
            lib.update_metadata(id, target)?;
            let names: Vec<&str> = changed.to_vec();
            let _ =
                store.set_provenance(id, &names, &best.sources.join("+"), Utc::now().timestamp());
            Ok(())
        })?;
        v["applied"] = json!(true);
        v["message"] = json!(format!("metadata updated from {}", best.sources.join(", ")));
    }
    Ok(ToolOutput::json(v))
}

/// Whether `path` is a file inside one of `allowed` (after resolving links and `..`).
pub fn path_allowed(
    path: &Path,
    allowed: &[std::path::PathBuf],
) -> Result<std::path::PathBuf, String> {
    let real = path
        .canonicalize()
        .map_err(|_| "that file does not exist".to_string())?;
    if !real.is_file() {
        return Err("that is not a file".into());
    }
    let ok = allowed
        .iter()
        .filter_map(|d| d.canonicalize().ok())
        .any(|d| real.starts_with(d));
    if ok {
        Ok(real)
    } else {
        Err("that file is outside the folders this server may read from".into())
    }
}

fn add_book_from_path(app: &Arc<App>, a: &Value) -> Result<ToolOutput, ToolError> {
    let path = path_allowed(
        Path::new(arg_str(a, "path").unwrap_or("")),
        &app.cfg.allow_dirs,
    )
    .map_err(ToolError)?;
    if !bookx_core::library::is_supported_file(&path) {
        return fail("this file format cannot be imported by this tool");
    }
    let dup = a
        .get("allow_duplicate")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let (outcome, book) = app.with_library(|lib, _| {
        let o = lib.add_path(&path, dup)?;
        let id = match o {
            AddOutcome::Added(i) | AddOutcome::Duplicate(i) => i,
        };
        Ok((o, summary(lib.get(id).expect("just added"))))
    })?;
    done(
        match outcome {
            AddOutcome::Added(_) => "book added".into(),
            AddOutcome::Duplicate(_) => {
                "this book is already in the library (nothing added)".into()
            }
        },
        Some(book),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::Chapter;

    fn bt(texts: &[&str]) -> BookText {
        BookText {
            chapters: texts
                .iter()
                .enumerate()
                .map(|(i, t)| Chapter {
                    index: i,
                    path: format!("c{i}.xhtml"),
                    title: format!("C{i}"),
                    text: t.to_string(),
                })
                .collect(),
            toc: vec![],
        }
    }

    #[test]
    fn locate_finds_text_with_character_offsets() {
        let t = bt(&["Hello wörld.\nSecond line here", "Another chapter"]);
        assert_eq!(locate(&t, None, "wörld"), Some((0, 6, 11, "wörld".into())));
        assert_eq!(
            locate(&t, None, "second   LINE").map(|x| (x.0, x.1)),
            Some((0, 13))
        );
        assert_eq!(locate(&t, Some(1), "Another chapter").map(|x| x.0), Some(1));
        assert!(locate(&t, Some(0), "Another").is_none());
        assert!(locate(&t, Some(9), "x").is_none());
        assert!(locate(&t, None, "   ").is_none());
    }

    #[test]
    fn allowed_paths_are_checked_after_resolving() {
        let d = std::env::temp_dir().join(format!("bookx-mcp-allow-{}", uuid::Uuid::new_v4()));
        let inside = d.join("ok");
        std::fs::create_dir_all(&inside).unwrap();
        std::fs::write(inside.join("a.epub"), b"x").unwrap();
        std::fs::write(d.join("secret.epub"), b"x").unwrap();
        let allowed = vec![inside.clone()];
        assert!(path_allowed(&inside.join("a.epub"), &allowed).is_ok());
        assert!(path_allowed(&inside.join("../secret.epub"), &allowed).is_err());
        assert!(path_allowed(&inside.join("missing.epub"), &allowed).is_err());
        assert!(path_allowed(&inside, &allowed).is_err());
        assert!(path_allowed(&d.join("secret.epub"), &[]).is_err());
        std::fs::remove_dir_all(d).ok();
    }

    #[test]
    fn names_and_merging() {
        assert_eq!(sort_name("Ann Lee"), "Lee, Ann");
        assert_eq!(sort_name("Lee, Ann"), "Lee, Ann");
        assert_eq!(sort_name("Plato"), "Plato");
        let cur = Metadata {
            title: "T".into(),
            publisher: "Mine".into(),
            ..Default::default()
        };
        let found = Metadata {
            title: "Other".into(),
            publisher: "Theirs".into(),
            language: "en".into(),
            tags: vec!["sf".into()],
            ..Default::default()
        };
        let fill = merge_found(&cur, &found, false);
        assert_eq!(
            (
                fill.publisher.as_str(),
                fill.language.as_str(),
                fill.title.as_str()
            ),
            ("Mine", "en", "T")
        );
        let rep = merge_found(&cur, &found, true);
        assert_eq!(rep.publisher, "Theirs");
        assert_eq!(changed_fields(&cur, &fill), vec!["language", "tags"]);
    }
}
