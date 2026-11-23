//! Further `bookx-cli` commands (X12): `catalog`, `fetch-news`, `polish`, `fonts`, `saved-search` and
//! `plugin run` (one editor action over the whole library).
//!
//! Custom columns are not in the library yet, so there is no command for them.

use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use bookx_core::archive::Archive;
use clap::Subcommand;

/// Most plugin/recipe/config bytes read from a file given on the command line.
const MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;

#[derive(Subcommand, Debug)]
pub enum SavedCmd {
    /// List the saved searches of the app (name and query)
    List {
        /// The app's config.json (default: the one the app uses)
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long)]
        json: bool,
    },
    /// Run a saved search against the library
    Run {
        name: String,
        #[arg(long)]
        config: Option<PathBuf>,
        #[arg(long, default_value = "title")]
        sort: String,
        #[arg(long)]
        desc: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Subcommand, Debug)]
pub enum PluginCmd {
    /// Run one action of an editor plugin on every book a search finds (default: all books).
    /// Shows what would change; `--apply` writes the books (the library must not be open in the app).
    Run {
        /// The plugin folder (plugin.toml and its .wasm)
        dir: PathBuf,
        /// The action id from plugin.toml
        action: String,
        /// Write the changes (without it nothing is touched)
        #[arg(long)]
        apply: bool,
        /// A library search; every book when empty
        query: Vec<String>,
        #[arg(long)]
        json: bool,
    },
}

// -- catalog ---------------------------------------------------------------------------------

/// The columns `catalog` can print, in the order they are printed.
pub const CATALOG_COLUMNS: &[&str] = &[
    "id",
    "title",
    "authors",
    "series",
    "index",
    "tags",
    "formats",
    "publisher",
    "language",
    "published",
    "size",
];

fn csv_field(s: &str) -> String {
    // a leading = + - @ would be run as a formula by spreadsheets
    let guard = s.starts_with(['=', '+', '-', '@', '\t', '\r']);
    let s = if guard {
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

pub fn catalog(
    root: &Path,
    query: &str,
    format: &str,
    columns: Option<&str>,
    output: Option<&Path>,
    out: &mut dyn Write,
) -> Result<i32> {
    if format == "epub" {
        let lib = crate::existing_library(root)?;
        let ids = lib.search(query, bookx_core::library::SortKey::Title, true);
        let books: Vec<_> = ids.iter().filter_map(|i| lib.get(*i)).collect();
        let Some(file) = output else {
            bail!("--format epub writes a file: give --output catalog.epub");
        };
        let bytes = bookx_core::catalog_epub::build(
            "Library catalogue",
            &books,
            &[
                "By title",
                "By author",
                "By series",
                "By tag",
                "No author",
                "No series",
                "No tag",
            ],
        )?;
        std::fs::write(file, bytes).with_context(|| format!("writing {}", file.display()))?;
        writeln!(
            out,
            "wrote a catalogue of {} book(s) to {}",
            books.len(),
            file.display()
        )?;
        return Ok(if books.is_empty() { 1 } else { 0 });
    }
    let cols: Vec<&str> = match columns {
        Some(c) => c
            .split(',')
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .collect(),
        None => CATALOG_COLUMNS.to_vec(),
    };
    for c in &cols {
        if !CATALOG_COLUMNS.contains(c) && !c.starts_with('#') {
            bail!("unknown column `{c}` (use: {})", CATALOG_COLUMNS.join(", "));
        }
    }
    if cols.is_empty() {
        bail!("no columns");
    }
    let lib = crate::existing_library(root)?;
    let ids = lib.search(query, bookx_core::library::SortKey::Title, true);
    let cell = |b: &bookx_core::library::Book, c: &str| -> serde_json::Value {
        use serde_json::json;
        match c {
            "id" => json!(b.id),
            "title" => json!(b.meta.title),
            "authors" => json!(b.meta.authors),
            "series" => json!(b.meta.series),
            "index" => json!(b.meta.series_index),
            "tags" => json!(b.meta.tags),
            "formats" => json!(
                b.all_formats()
                    .iter()
                    .map(|f| f.format.clone())
                    .collect::<Vec<_>>()
            ),
            "publisher" => json!(b.meta.publisher),
            "language" => json!(b.meta.language),
            "published" => json!(b.meta.published),
            "size" => json!(b.size),
            c if c.starts_with('#') => json!(b.custom_value(&c[1..])),
            _ => serde_json::Value::Null,
        }
    };
    let books: Vec<_> = ids.iter().filter_map(|i| lib.get(*i)).collect();
    match format {
        "json" => {
            let rows: Vec<_> = books
                .iter()
                .map(|b| {
                    cols.iter()
                        .map(|c| (c.to_string(), cell(b, c)))
                        .collect::<serde_json::Map<_, _>>()
                })
                .collect();
            writeln!(out, "{}", serde_json::to_string_pretty(&rows)?)?;
        }
        "csv" => {
            writeln!(out, "{}", cols.join(","))?;
            for b in &books {
                let line: Vec<String> = cols
                    .iter()
                    .map(|c| match cell(b, c) {
                        serde_json::Value::String(s) => csv_field(&s),
                        serde_json::Value::Array(a) => csv_field(
                            &a.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join("; "),
                        ),
                        serde_json::Value::Null => String::new(),
                        v => v.to_string(),
                    })
                    .collect();
                writeln!(out, "{}", line.join(","))?;
            }
        }
        other => bail!("unknown format `{other}` (use csv, json or epub)"),
    }
    Ok(if books.is_empty() { 1 } else { 0 })
}

// -- fonts -----------------------------------------------------------------------------------

pub fn fonts(file: &Path, json: bool, out: &mut dyn Write) -> Result<i32> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("book");
    let archive = crate::read_archive(name, &bytes)?;
    let book = bookx_core::book::OpenBook::from_archive(archive)?;
    let list = bookx_editing::fonts_manage::list_fonts(&book);
    let rep = bookx_editing::fonts::analyze_fonts(&book);
    if json {
        let rows: Vec<_> = list
            .iter()
            .map(|f| {
                serde_json::json!({
                    "path": f.path, "family": f.display_family(), "style": f.file_style,
                    "bytes": f.bytes, "used_chars": f.used_chars, "unused": f.unused,
                    "missing_glyphs": f.missing_glyphs, "embedding": format!("{:?}", f.embedding),
                })
            })
            .collect();
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "fonts": rows,
                "undeclared_families": rep.undeclared_families,
                "orphan_files": rep.orphan_files,
                "missing_files": rep.missing_files.iter().map(|m| &m.2).collect::<Vec<_>>(),
            }))?
        )?;
    } else {
        if list.is_empty() {
            writeln!(out, "no embedded fonts")?;
        }
        for f in &list {
            writeln!(
                out,
                "{}\t{} {}\t{} bytes\t{} chars used{}{}",
                f.path,
                f.display_family(),
                f.file_style,
                f.bytes,
                f.used_chars,
                if f.unused { "\tunused" } else { "" },
                if f.missing_glyphs > 0 {
                    format!("\t{} glyphs missing", f.missing_glyphs)
                } else {
                    String::new()
                }
            )?;
        }
        for (fam, n) in &rep.undeclared_families {
            writeln!(out, "not embedded: {fam} ({n} chars)")?;
        }
        for o in &rep.orphan_files {
            writeln!(out, "orphan font file: {o}")?;
        }
    }
    Ok(0)
}

// -- polish ----------------------------------------------------------------------------------

/// Applies the editor's automatic fixes to a book file and writes the result. Exit 1 when problems remain.
pub fn polish(file: &Path, output: &Path, json: bool, out: &mut dyn Write) -> Result<i32> {
    let bytes = std::fs::read(file).with_context(|| format!("reading {}", file.display()))?;
    let name = file.file_name().and_then(|n| n.to_str()).unwrap_or("book");
    let archive = crate::read_archive(name, &bytes)?;
    let (book, report) =
        bookx_editing::fixes::fix_archive(archive, &bookx_editing::fixes::FixOptions::default())?;
    book.archive.save(output)?;
    if json {
        writeln!(out, "{}", serde_json::to_string_pretty(&report)?)?;
    } else {
        for f in &report.fixes {
            writeln!(out, "fixed {}: {}", f.file, f.description)?;
        }
        for f in &report.failures {
            writeln!(out, "could not fix {f}")?;
        }
        for r in &report.remaining_errors {
            writeln!(out, "still wrong: {r}")?;
        }
        writeln!(
            out,
            "{} fix(es), wrote {}",
            report.fixes.len(),
            output.display()
        )?;
    }
    Ok(
        if report.remaining_errors.is_empty() && report.failures.is_empty() {
            0
        } else {
            1
        },
    )
}

// -- saved searches --------------------------------------------------------------------------

fn default_config() -> Option<PathBuf> {
    directories::ProjectDirs::from("com", "adhadse", "Bookx")
        .map(|d| d.config_dir().join("config.json"))
}

/// `(name, query)` pairs from the app's config.json (`saved_searches`).
pub fn read_saved(config: Option<&Path>) -> Result<Vec<(String, String)>> {
    let path = config
        .map(Path::to_path_buf)
        .or_else(default_config)
        .ok_or_else(|| anyhow!("no config file known"))?;
    let len = std::fs::metadata(&path)
        .with_context(|| format!("reading {}", path.display()))?
        .len();
    if len > MAX_INPUT_BYTES {
        bail!("{} is larger than {MAX_INPUT_BYTES} bytes", path.display());
    }
    let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;
    Ok(v.get("saved_searches")
        .and_then(|s| s.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|e| {
                    Some((
                        e.get("name")?.as_str()?.to_string(),
                        e.get("query")?.as_str()?.to_string(),
                    ))
                })
                .collect()
        })
        .unwrap_or_default())
}

pub fn saved(cmd: SavedCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    match cmd {
        SavedCmd::List { config, json } => {
            let list = read_saved(config.as_deref())?;
            if json {
                let v: Vec<_> = list
                    .iter()
                    .map(|(n, q)| serde_json::json!({"name": n, "query": q}))
                    .collect();
                writeln!(out, "{}", serde_json::to_string_pretty(&v)?)?;
            } else {
                for (n, q) in &list {
                    writeln!(out, "{n}\t{q}")?;
                }
            }
            Ok(if list.is_empty() { 1 } else { 0 })
        }
        SavedCmd::Run {
            name,
            config,
            sort,
            desc,
            json,
        } => {
            let list = read_saved(config.as_deref())?;
            let (_, query) = list
                .iter()
                .find(|(n, _)| n.eq_ignore_ascii_case(&name))
                .ok_or_else(|| anyhow!("no saved search called `{name}`"))?;
            let low = query.to_ascii_lowercase();
            if low.contains("search:") || low.contains("vl:") {
                bail!("`{name}` refers to other saved searches; run it in the app");
            }
            crate::search(root, query, &sort, desc, json, out)
        }
    }
}

// -- fetch-news ------------------------------------------------------------------------------

/// Builds the EPUB of a recipe (TOML or JSON) and writes it. Needs the network (feeds).
pub fn fetch_news(
    recipe_file: &Path,
    output: &Path,
    fetcher: &dyn bookx_sources::fetch::Fetcher,
    out: &mut dyn Write,
) -> Result<i32> {
    let len = std::fs::metadata(recipe_file)
        .with_context(|| format!("reading {}", recipe_file.display()))?
        .len();
    if len > MAX_INPUT_BYTES {
        bail!("the recipe is larger than {MAX_INPUT_BYTES} bytes");
    }
    let text = std::fs::read_to_string(recipe_file)?;
    let recipe = if recipe_file.extension().is_some_and(|e| e == "json") {
        bookx_sources::recipe::Recipe::from_json(&text)?
    } else {
        bookx_sources::recipe::Recipe::from_toml(&text)?
    };
    let news = bookx_sources::recipe::build_news_epub(&recipe, fetcher, chrono::Utc::now())?;
    news.archive.save(output)?;
    for e in &news.errors {
        writeln!(out, "warning: {e}")?;
    }
    writeln!(out, "{}: wrote {}", news.title, output.display())?;
    Ok(0)
}

// -- plugin run ------------------------------------------------------------------------------

pub fn plugin(cmd: PluginCmd, root: &Path, out: &mut dyn Write) -> Result<i32> {
    let PluginCmd::Run {
        dir,
        action,
        apply,
        query,
        json,
    } = cmd;
    let text = {
        let p = dir.join("plugin.toml");
        let len = std::fs::metadata(&p)
            .with_context(|| format!("reading {}", p.display()))?
            .len();
        if len > MAX_INPUT_BYTES {
            bail!("plugin.toml is larger than {MAX_INPUT_BYTES} bytes");
        }
        std::fs::read_to_string(&p)?
    };
    let manifest = bookx_plugin::Manifest::parse(&text)?;
    let plugin = bookx_plugin::Plugin::load(&dir, manifest, bookx_plugin::Host::default())?;
    let q = query.join(" ");
    let mut g = if apply {
        crate::libtools::open_for_write(root)?
    } else {
        bookx_store::guard::open_library_guarded(root, "Bookx CLI", true)?
    };
    if !bookx_store::has_library(root) {
        bail!(
            "no library at {} (add books first, or pass --library)",
            root.display()
        );
    }
    let lib = &mut g.opened.library;
    let ids = lib.search(&q, bookx_core::library::SortKey::Title, true);
    let (mut changed_books, mut failed) = (0usize, 0usize);
    let mut rows = Vec::new();
    for id in ids {
        let Some(book) = lib.get(id) else { continue };
        let title = book.meta.title.clone();
        let path = lib.path_for(book, bookx_core::formats::Target::Reader);
        let is_epub = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("epub"));
        let step: Result<(Vec<String>, Option<Archive>)> = (|| {
            if !is_epub {
                bail!("not an EPUB");
            }
            let archive = Archive::open(&path)?;
            let o = bookx_plugin::batch::run_on_archive(&plugin, &action, &archive, &action)?;
            let keep = (!o.changed_files.is_empty()).then_some(o.archive);
            Ok((o.changed_files, keep))
        })();
        match step {
            Ok((files, new)) => {
                if !files.is_empty() {
                    changed_books += 1;
                }
                if let (true, Some(a)) = (apply, new) {
                    lib.replace_archive(id, &a)?;
                }
                rows.push(serde_json::json!({"id": id, "title": title, "changed_files": files}));
            }
            Err(e) => {
                failed += 1;
                rows.push(serde_json::json!({"id": id, "title": title, "error": format!("{e:#}")}));
            }
        }
    }
    if json {
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "applied": apply, "changed": changed_books, "failed": failed, "books": rows,
            }))?
        )?;
    } else {
        for r in &rows {
            let t = r["title"].as_str().unwrap_or("");
            match (r.get("error"), r["changed_files"].as_array()) {
                (Some(e), _) => {
                    writeln!(out, "{}\t{t}\terror: {}", r["id"], e.as_str().unwrap_or(""))?
                }
                (_, Some(f)) if !f.is_empty() => writeln!(
                    out,
                    "{}\t{t}\t{} file(s) {}",
                    r["id"],
                    f.len(),
                    if apply { "changed" } else { "would change" }
                )?,
                _ => {}
            }
        }
        writeln!(
            out,
            "{changed_books} book(s) {}, {failed} failed{}",
            if apply { "changed" } else { "would change" },
            if apply {
                ""
            } else {
                " (nothing written; use --apply)"
            }
        )?;
    }
    Ok(if failed > 0 { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bookx_sources::fetch::MockFetcher;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("bookx-more-{tag}-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn csv_cells_cannot_run_as_spreadsheet_formulas() {
        assert_eq!(csv_field("plain"), "plain");
        assert_eq!(csv_field("a,b"), "\"a,b\"");
        assert_eq!(csv_field("say \"x\""), "\"say \"\"x\"\"\"");
        assert_eq!(
            csv_field("=HYPERLINK(\"http://x\")"),
            "\"'=HYPERLINK(\"\"http://x\"\")\""
        );
        assert_eq!(csv_field("+1"), "'+1");
        assert_eq!(csv_field("@SUM"), "'@SUM");
    }

    #[test]
    fn fetch_news_writes_an_epub_from_mocked_feeds() {
        let d = tmp("news");
        let rss = "<rss version=\"2.0\"><channel><title>T</title><item><title>First story</title><link>https://h.example/a</link><description>&lt;p&gt;Body words&lt;/p&gt;</description></item></channel></rss>";
        let f = MockFetcher::new().with_body("https://h.example/f", rss);
        let mut r = bookx_sources::recipe::Recipe::new("Daily", &["https://h.example/f"]);
        r.oldest_days = 0;
        let rf = d.join("daily.toml");
        std::fs::write(&rf, r.to_toml().unwrap()).unwrap();
        let out_file = d.join("daily.epub");
        let mut out = Vec::new();
        assert_eq!(fetch_news(&rf, &out_file, &f, &mut out).unwrap(), 0);
        let a = Archive::open(&out_file).unwrap();
        assert!(a.names().iter().any(|n| n.ends_with(".opf")));
        assert!(String::from_utf8(out).unwrap().contains("wrote"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn fetch_news_refuses_a_huge_or_broken_recipe() {
        let d = tmp("news-bad");
        let f = MockFetcher::new();
        let bad = d.join("bad.toml");
        std::fs::write(&bad, "this is = = not toml").unwrap();
        assert!(fetch_news(&bad, &d.join("o.epub"), &f, &mut Vec::new()).is_err());
        let huge = d.join("huge.toml");
        std::fs::write(&huge, vec![b'#'; (MAX_INPUT_BYTES + 1) as usize]).unwrap();
        let e = fetch_news(&huge, &d.join("o.epub"), &f, &mut Vec::new()).unwrap_err();
        assert!(format!("{e}").contains("larger"));
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn saved_searches_are_read_from_the_config_and_junk_is_skipped() {
        let d = tmp("saved");
        let c = d.join("config.json");
        std::fs::write(
            &c,
            r#"{"saved_searches":[{"name":"Unread","query":"progress:0"},{"name":5},{"name":"Q"}],"other":1}"#,
        )
        .unwrap();
        assert_eq!(
            read_saved(Some(&c)).unwrap(),
            vec![("Unread".to_string(), "progress:0".to_string())]
        );
        std::fs::write(&c, "{}").unwrap();
        assert!(read_saved(Some(&c)).unwrap().is_empty());
        std::fs::write(&c, "not json").unwrap();
        assert!(read_saved(Some(&c)).is_err());
        assert!(read_saved(Some(&d.join("missing.json"))).is_err());
        let _ = std::fs::remove_dir_all(d);
    }
}
