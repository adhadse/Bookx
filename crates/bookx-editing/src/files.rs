//! File-list operations of the editor without a GUI: rename with reference repair, bulk rename, delete,
//! add, new files from templates, link stylesheets, export (CE-023..CE-025, CE-034, CE-035, CE-039..CE-041).

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::{bail, Context, Result};
use bookx_core::book::OpenBook;
use bookx_core::opf::SpineItem;
use bookx_core::util::{
    extension, file_name, join_path, media_type_for, parent_dir, relative_href, xml_escape,
};

use crate::markup::{is_css_path, is_markup_path, rename_paths, scan, Event};

/// Paths the package depends on; they can be neither renamed nor deleted.
pub fn is_protected(book: &OpenBook, path: &str) -> bool {
    path == book.package.opf_path || path.starts_with("META-INF/") || path == "mimetype"
}

/// Renames files and repairs every reference to them (`href`, `src`, `url()`, fragments, OPF, NCX).
/// Returns the number of files whose text changed.
pub fn rename_files(book: &mut OpenBook, map: &HashMap<String, String>) -> Result<usize> {
    let map: HashMap<String, String> = map
        .iter()
        .filter(|(a, b)| a != b)
        .map(|(a, b)| (a.clone(), b.clone()))
        .collect();
    if map.is_empty() {
        return Ok(0);
    }
    let mut targets = HashSet::new();
    for (from, to) in &map {
        if !book.archive.contains(from) {
            bail!("{from} is not in the book");
        }
        if is_protected(book, from) {
            bail!("{from} is required by the package and cannot be renamed");
        }
        if to.trim().is_empty() || file_name(to).is_empty() {
            bail!("the new name of {from} is empty");
        }
        if !targets.insert(to.clone()) {
            bail!("two files would be named {to}");
        }
        if book.archive.contains(to) && !map.contains_key(to) {
            bail!("a file named {to} already exists");
        }
    }
    let changed = rename_paths(&mut book.archive, &map);
    book.reload_package()?;
    Ok(changed)
}

/// `a/b/old.xhtml` + new file name -> `a/b/new.xhtml`.
pub fn renamed_in_place(path: &str, new_name: &str) -> String {
    join_path(parent_dir(path), new_name.trim())
}

/// Bulk rename (CE-024): `prefix` + running number (zero padded to the widest number) + the old extension,
/// in the order of `paths`.
pub fn bulk_rename_plan(paths: &[String], prefix: &str, start: u32) -> HashMap<String, String> {
    let last = start as usize + paths.len().saturating_sub(1);
    let width = last.to_string().len();
    paths
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let ext = extension(p);
            let dot = if ext.is_empty() { "" } else { "." };
            let name = format!("{prefix}{:0width$}{dot}{ext}", start as usize + i);
            (p.clone(), join_path(parent_dir(p), &name))
        })
        .collect()
}

/// Gives every file a new extension (CE-025).
pub fn change_extension_plan(paths: &[String], ext: &str) -> HashMap<String, String> {
    let ext = ext.trim().trim_start_matches('.');
    paths
        .iter()
        .map(|p| {
            let stem = file_name(p)
                .rsplit_once('.')
                .map(|s| s.0)
                .unwrap_or(file_name(p));
            let name = if ext.is_empty() {
                stem.to_string()
            } else {
                format!("{stem}.{ext}")
            };
            (p.clone(), join_path(parent_dir(p), &name))
        })
        .collect()
}

/// Deletes files: manifest and spine entries go, the archive entries go; references in other files are left
/// for the checker. Returns the paths actually deleted (protected files are skipped).
pub fn delete_files(book: &mut OpenBook, paths: &[String]) -> Result<Vec<String>> {
    let mut done = Vec::new();
    for p in paths {
        if is_protected(book, p) || !book.archive.contains(p) {
            continue;
        }
        if let Some(id) = book.package.item_by_path(p).map(|i| i.id.clone()) {
            let pkg = book.package.clone();
            pkg.remove_manifest_item(&mut book.archive, &id)?;
            book.reload_package()?;
        }
        book.archive.remove(p);
        done.push(p.clone());
    }
    book.reload_package()?;
    Ok(done)
}

fn folder_for(book: &OpenBook, media: &str) -> String {
    if let Some(i) = book.package.manifest.iter().find(|i| i.media_type == media) {
        return parent_dir(&book.package.item_path(i)).to_string();
    }
    let sub = if media.starts_with("image/") {
        "images"
    } else if media == "text/css" {
        "styles"
    } else if media.starts_with("font/") || media.contains("font") {
        "fonts"
    } else {
        "text"
    };
    join_path(book.package.base_dir(), sub)
}

/// `name`, or `name-2`, `name-3`... when the path is taken.
pub fn unique_path(book: &OpenBook, dir: &str, name: &str) -> String {
    let mut abs = join_path(dir, name);
    let mut n = 1;
    while book.archive.contains(&abs) {
        n += 1;
        let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
        let dot = if ext.is_empty() { "" } else { "." };
        abs = join_path(dir, &format!("{stem}-{n}{dot}{ext}"));
    }
    abs
}

/// Adds a file to the archive and the manifest. XHTML goes into the reading order: at the end, or right after
/// `after` (a path in the spine). Returns the new path.
pub fn add_file(
    book: &mut OpenBook,
    name: &str,
    data: Vec<u8>,
    after: Option<&str>,
) -> Result<String> {
    let media = media_type_for(name);
    let dir = folder_for(book, media);
    let abs = unique_path(book, &dir, file_name(name));
    let base: String = file_name(&abs)
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '_' })
        .collect();
    let mut id = base.clone();
    let mut k = 1;
    while book.package.item_by_id(&id).is_some() {
        k += 1;
        id = format!("{base}_{k}");
    }
    book.archive.set(&abs, data);
    let pkg = book.package.clone();
    pkg.add_manifest_item(&mut book.archive, &id, &abs, media, "")?;
    book.reload_package()?;
    if media == "application/xhtml+xml" {
        let mut order = book.package.spine.clone();
        let item = SpineItem {
            idref: id,
            linear: true,
        };
        let at = after
            .and_then(|a| book.package.item_by_path(a).map(|i| i.id.clone()))
            .and_then(|aid| order.iter().position(|s| s.idref == aid))
            .map(|i| i + 1)
            .unwrap_or(order.len());
        order.insert(at, item);
        let pkg = book.package.clone();
        pkg.set_spine(&mut book.archive, &order)?;
        book.reload_package()?;
    }
    Ok(abs)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Template {
    BlankXhtml,
    Chapter,
    TitlePage,
    BlankCss,
}

impl Template {
    pub const ALL: [Template; 4] = [
        Template::BlankXhtml,
        Template::Chapter,
        Template::TitlePage,
        Template::BlankCss,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Template::BlankXhtml => "blank",
            Template::Chapter => "chapter",
            Template::TitlePage => "titlepage",
            Template::BlankCss => "css",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Template::BlankXhtml => "Blank page (XHTML)",
            Template::Chapter => "Chapter with a heading",
            Template::TitlePage => "Title page",
            Template::BlankCss => "Stylesheet (CSS)",
        }
    }

    pub fn from_id(id: &str) -> Option<Template> {
        Template::ALL.into_iter().find(|t| t.id() == id)
    }

    pub fn extension(self) -> &'static str {
        match self {
            Template::BlankCss => "css",
            _ => "xhtml",
        }
    }

    /// File content; `title` and `lang` fill the page.
    pub fn content(self, title: &str, lang: &str) -> String {
        let t = xml_escape(title);
        let page = |body: &str| {
            format!(
                "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xml:lang=\"{lang}\" lang=\"{lang}\">\n<head>\n  <title>{t}</title>\n</head>\n<body>\n{body}</body>\n</html>\n"
            )
        };
        match self {
            Template::BlankXhtml => page("  <p></p>\n"),
            Template::Chapter => page(&format!("  <h1>{t}</h1>\n  <p></p>\n")),
            Template::TitlePage => page(&format!(
                "  <div style=\"text-align: center\">\n    <h1>{t}</h1>\n  </div>\n"
            )),
            Template::BlankCss => "/* Styles */\n".to_string(),
        }
    }
}

/// Normalises a name typed by the user: adds the template's extension when missing.
pub fn new_file_name(typed: &str, template: Template) -> Option<String> {
    let n = typed.trim();
    if n.is_empty() || n.contains('/') || n.contains('\\') || n.starts_with('.') {
        return None;
    }
    let want = template.extension();
    let ext = extension(n);
    let ok = match template {
        Template::BlankCss => ext == "css",
        _ => matches!(ext.as_str(), "xhtml" | "html" | "htm"),
    };
    Some(if ok {
        n.to_string()
    } else {
        format!("{n}.{want}")
    })
}

/// Creates a blank file from a template (CE-039). XHTML files get the book's stylesheets linked when
/// `link_styles` is set. Returns the new path.
pub fn create_from_template(
    book: &mut OpenBook,
    name: &str,
    template: Template,
    after: Option<&str>,
    link_styles: bool,
) -> Result<String> {
    let name = new_file_name(name, template).context("that is not a valid file name")?;
    let title = name
        .rsplit_once('.')
        .map(|s| s.0)
        .unwrap_or(&name)
        .replace(['-', '_'], " ");
    let lang = if book.metadata().language.is_empty() {
        "en".to_string()
    } else {
        book.metadata().language.clone()
    };
    let content = template.content(&title, &lang);
    let path = add_file(book, &name, content.into_bytes(), after)?;
    if link_styles && template != Template::BlankCss {
        let css: Vec<String> = book
            .archive
            .names()
            .iter()
            .filter(|n| is_css_path(n))
            .cloned()
            .collect();
        link_stylesheets(book, std::slice::from_ref(&path), &css, false)?;
    }
    Ok(path)
}

/// Links stylesheets into the `<head>` of markup files, in the given order (CE-041). With `remove_existing`
/// the old stylesheet links go first. Returns the number of files changed.
pub fn link_stylesheets(
    book: &mut OpenBook,
    html: &[String],
    css: &[String],
    remove_existing: bool,
) -> Result<usize> {
    let mut changed = 0;
    for h in html {
        if !is_markup_path(h) {
            continue;
        }
        let Some(src) = book.archive.get_string(h) else {
            continue;
        };
        let out = with_stylesheet_links(&src, h, css, remove_existing);
        if out != src {
            book.archive.set_string(h, &out);
            changed += 1;
        }
    }
    Ok(changed)
}

fn is_stylesheet_link(tag: &str) -> bool {
    crate::markup::tag_attr(tag, "rel")
        .is_some_and(|r| r.to_ascii_lowercase().contains("stylesheet"))
}

/// The pure part of [`link_stylesheets`]: new text of the file `at`.
pub fn with_stylesheet_links(src: &str, at: &str, css: &[String], remove_existing: bool) -> String {
    let events = scan(src);
    let mut text = src.to_string();
    if remove_existing {
        let mut cut: Vec<std::ops::Range<usize>> = events
            .iter()
            .filter_map(|e| match e {
                Event::Open { name, range, .. }
                    if name == "link" && is_stylesheet_link(&src[range.clone()]) =>
                {
                    Some(range.clone())
                }
                _ => None,
            })
            .collect();
        cut.sort_by_key(|r| std::cmp::Reverse(r.start));
        for r in cut {
            let mut start = r.start;
            while start > 0 && text.as_bytes()[start - 1] == b' ' {
                start -= 1;
            }
            let mut end = r.end;
            if text.as_bytes().get(end) == Some(&b'\n') {
                end += 1;
            }
            if start > 0 && text.as_bytes()[start - 1] == b'\n' || start == 0 {
                text.replace_range(start..end, "");
            } else {
                text.replace_range(r, "");
            }
        }
    }
    let events = scan(&text);
    let head_close = events.iter().find_map(|e| match e {
        Event::Close { name, range } if name == "head" => Some(range.start),
        _ => None,
    });
    let Some(at_pos) = head_close else {
        return text;
    };
    let existing: HashSet<String> = events
        .iter()
        .filter_map(|e| match e {
            Event::Open { name, range, .. } if name == "link" => {
                let tag = &text[range.clone()];
                if is_stylesheet_link(tag) {
                    crate::markup::tag_attr(tag, "href")
                } else {
                    None
                }
            }
            _ => None,
        })
        .collect();
    let mut add = String::new();
    for c in css {
        let href = relative_href(at, c);
        if existing.contains(&href) {
            continue;
        }
        add.push_str(&format!(
            "  <link rel=\"stylesheet\" type=\"text/css\" href=\"{}\"/>\n",
            xml_escape(&href)
        ));
    }
    if add.is_empty() {
        return text;
    }
    let line_start = text[..at_pos].rfind('\n').map(|p| p + 1).unwrap_or(at_pos);
    let ins = if text[line_start..at_pos].trim().is_empty() {
        line_start
    } else {
        add.insert(0, '\n');
        at_pos
    };
    text.insert_str(ins, &add);
    text
}

/// Writes the files into `dir` under their plain names (clashes get a number). Returns the written paths.
pub fn export_files(
    book: &OpenBook,
    paths: &[String],
    dir: &Path,
) -> Result<Vec<std::path::PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut out = Vec::new();
    for p in paths {
        let data = book
            .archive
            .get(p)
            .with_context(|| format!("{p} is missing"))?;
        let name = file_name(p);
        let mut target = dir.join(name);
        let mut n = 1;
        while target.exists() {
            n += 1;
            let (stem, ext) = name.rsplit_once('.').unwrap_or((name, ""));
            let dot = if ext.is_empty() { "" } else { "." };
            target = dir.join(format!("{stem}-{n}{dot}{ext}"));
        }
        std::fs::write(&target, data)?;
        out.push(target);
    }
    Ok(out)
}

/// Score of `query` against one string: 0 = no match. Prefix > substring > scattered letters.
fn fuzzy_score(query: &str, text: &str) -> u32 {
    let (q, t) = (query.to_lowercase(), text.to_lowercase());
    if t.starts_with(&q) {
        return 1000 - t.len().min(500) as u32;
    }
    if let Some(at) = t.find(&q) {
        return 600 - at.min(300) as u32;
    }
    let mut it = t.chars().enumerate();
    let mut last: Option<usize> = None;
    let mut gaps = 0usize;
    for c in q.chars() {
        match it.find(|(_, tc)| *tc == c) {
            Some((i, _)) => {
                if let Some(l) = last {
                    gaps += i - l - 1;
                }
                last = Some(i);
            }
            None => return 0,
        }
    }
    300u32.saturating_sub(gaps.min(250) as u32).max(1)
}

/// Quick open (CE-010): the files whose name (preferred) or path matches `query`, best first. An empty query
/// lists everything in the given order.
pub fn quick_open(query: &str, files: &[String]) -> Vec<String> {
    let q = query.trim();
    if q.is_empty() {
        return files.to_vec();
    }
    let mut scored: Vec<(u32, &String)> = files
        .iter()
        .filter_map(|f| {
            let by_name = fuzzy_score(q, file_name(f)) * 2;
            let by_path = fuzzy_score(q, f);
            let s = by_name.max(by_path);
            (s > 0).then_some((s, f))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    scored.into_iter().map(|(_, f)| f.clone()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    fn book_with_images() -> OpenBook {
        let mut b = make_book(&["<p>one</p>", "<p>two</p>"]);
        for n in ["a.png", "data.png"] {
            let p = format!("OEBPS/images/{n}");
            b.archive.set(&p, crate::testutil::png(4, 4));
            let pkg = b.package.clone();
            pkg.add_manifest_item(&mut b.archive, n, &p, "image/png", "")
                .unwrap();
        }
        b.reload_package().unwrap();
        let ch = b.spine_paths()[0].clone();
        let src = b.archive.get_string(&ch).unwrap();
        b.archive.set_string(
            &ch,
            &src.replace(
                "</body>",
                "<img src=\"images/a.png\"/><img src=\"images/data.png\"/><a href=\"chapter-2.xhtml#top\">x</a></body>",
            ),
        );
        b
    }

    #[test]
    fn renaming_a_png_leaves_a_longer_name_alone() {
        let mut b = book_with_images();
        let mut m = HashMap::new();
        m.insert(
            "OEBPS/images/a.png".to_string(),
            "OEBPS/images/b.png".to_string(),
        );
        rename_files(&mut b, &m).unwrap();
        let ch = b.archive.get_string(&b.spine_paths()[0]).unwrap();
        assert!(ch.contains("images/b.png"));
        assert!(ch.contains("images/data.png"));
        assert!(!ch.contains("images/a.png"));
        assert!(b.archive.contains("OEBPS/images/b.png"));
        assert!(b.package.item_by_path("OEBPS/images/b.png").is_some());
        assert!(b.package.item_by_path("OEBPS/images/data.png").is_some());
    }

    #[test]
    fn css_urls_and_fragments_follow_a_rename() {
        let mut b = book_with_images();
        b.archive.set_string(
            "OEBPS/style.css",
            "p { background: url(images/a.png) } q { background: url('images/data.png') }",
        );
        let first = b.spine_paths()[0].clone();
        let mut m = HashMap::new();
        m.insert(
            "OEBPS/images/a.png".to_string(),
            "OEBPS/images/b.png".to_string(),
        );
        m.insert(
            "OEBPS/chapter-2.xhtml".to_string(),
            "OEBPS/two.xhtml".to_string(),
        );
        rename_files(&mut b, &m).unwrap();
        let css = b.archive.get_string("OEBPS/style.css").unwrap();
        assert!(css.contains("url(images/b.png)"));
        assert!(css.contains("images/data.png"));
        let ch = b.archive.get_string(&first).unwrap();
        assert!(ch.contains("href=\"two.xhtml#top\""), "{ch}");
        assert!(b.package.item_by_path("OEBPS/two.xhtml").is_some());
    }

    #[test]
    fn renames_are_validated() {
        let mut b = book_with_images();
        let mut m = HashMap::new();
        m.insert(
            "OEBPS/images/a.png".to_string(),
            "OEBPS/images/data.png".to_string(),
        );
        assert!(rename_files(&mut b, &m).is_err());
        let mut m = HashMap::new();
        m.insert(b.package.opf_path.clone(), "x.opf".to_string());
        assert!(rename_files(&mut b, &m).is_err());
        let mut m = HashMap::new();
        m.insert("nope.png".to_string(), "x.png".to_string());
        assert!(rename_files(&mut b, &m).is_err());
        // swapping two names is fine
        let mut m = HashMap::new();
        m.insert(
            "OEBPS/images/a.png".to_string(),
            "OEBPS/images/data.png".to_string(),
        );
        m.insert(
            "OEBPS/images/data.png".to_string(),
            "OEBPS/images/a.png".to_string(),
        );
        assert!(rename_files(&mut b, &m).is_ok());
    }

    #[test]
    fn bulk_names_are_numbered_padded_and_keep_the_extension() {
        let paths: Vec<String> = (0..12).map(|i| format!("OEBPS/t/x{i}.xhtml")).collect();
        let plan = bulk_rename_plan(&paths, "part-", 1);
        assert_eq!(plan["OEBPS/t/x0.xhtml"], "OEBPS/t/part-01.xhtml");
        assert_eq!(plan["OEBPS/t/x11.xhtml"], "OEBPS/t/part-12.xhtml");
        let plan = bulk_rename_plan(&["a/b".to_string()], "n", 7);
        assert_eq!(plan["a/b"], "a/n7");
        let ext = change_extension_plan(&["d/a.htm".to_string(), "d/b".to_string()], ".xhtml");
        assert_eq!(ext["d/a.htm"], "d/a.xhtml");
        assert_eq!(ext["d/b"], "d/b.xhtml");
    }

    #[test]
    fn bulk_rename_applies_through_rename_files() {
        let mut b = book_with_images();
        let spine = b.spine_paths();
        let plan = bulk_rename_plan(&spine, "sec", 1);
        rename_files(&mut b, &plan).unwrap();
        assert_eq!(
            b.spine_paths(),
            vec!["OEBPS/sec1.xhtml", "OEBPS/sec2.xhtml"]
        );
        let ch = b.archive.get_string("OEBPS/sec1.xhtml").unwrap();
        assert!(ch.contains("sec2.xhtml#top"));
    }

    #[test]
    fn deleting_skips_protected_files_and_cleans_the_package() {
        let mut b = book_with_images();
        let opf = b.package.opf_path.clone();
        let done = delete_files(
            &mut b,
            &[
                "OEBPS/images/a.png".to_string(),
                opf.clone(),
                "META-INF/container.xml".to_string(),
            ],
        )
        .unwrap();
        assert_eq!(done, vec!["OEBPS/images/a.png"]);
        assert!(b.archive.contains(&opf));
        assert!(b.package.item_by_path("OEBPS/images/a.png").is_none());
        assert!(b.package.item_by_path("OEBPS/images/data.png").is_some());
    }

    #[test]
    fn new_names_get_the_template_extension() {
        assert_eq!(
            new_file_name("intro", Template::Chapter).unwrap(),
            "intro.xhtml"
        );
        assert_eq!(
            new_file_name("intro.html", Template::Chapter).unwrap(),
            "intro.html"
        );
        assert_eq!(new_file_name("x", Template::BlankCss).unwrap(), "x.css");
        assert_eq!(
            new_file_name("x.xhtml", Template::BlankCss).unwrap(),
            "x.xhtml.css"
        );
        assert!(new_file_name("a/b", Template::Chapter).is_none());
        assert!(new_file_name("  ", Template::Chapter).is_none());
        for t in Template::ALL {
            assert_eq!(Template::from_id(t.id()), Some(t));
        }
    }

    #[test]
    fn a_new_page_enters_the_reading_order_after_the_chosen_file_and_links_styles() {
        let mut b = make_book(&["<p>one</p>", "<p>two</p>"]);
        b.archive.set_string("OEBPS/styles/main.css", "p{}");
        let pkg = b.package.clone();
        pkg.add_manifest_item(
            &mut b.archive,
            "css",
            "OEBPS/styles/main.css",
            "text/css",
            "",
        )
        .unwrap();
        b.reload_package().unwrap();
        let first = b.spine_paths()[0].clone();
        let p = create_from_template(&mut b, "interlude", Template::Chapter, Some(&first), true)
            .unwrap();
        let spine = b.spine_paths();
        assert_eq!(spine[1], p);
        assert_eq!(spine.len(), 3);
        let text = b.archive.get_string(&p).unwrap();
        assert!(text.contains("<h1>interlude</h1>"));
        assert!(
            text.contains("href=\"../styles/main.css\"") || text.contains("main.css"),
            "{text}"
        );
        assert!(bookx_core::util::parse_xml(&text).is_ok());
        let css = create_from_template(&mut b, "extra", Template::BlankCss, None, true).unwrap();
        assert!(css.ends_with(".css"));
        assert_eq!(b.spine_paths().len(), 3);
    }

    #[test]
    fn linking_stylesheets_is_idempotent_ordered_and_can_replace() {
        let src = "<html><head>\n  <title>t</title>\n  <link rel=\"stylesheet\" href=\"old.css\"/>\n</head><body/></html>";
        let css = vec!["d/a.css".to_string(), "d/b.css".to_string()];
        let once = with_stylesheet_links(src, "d/p.xhtml", &css, false);
        assert!(once.contains("old.css"));
        assert!(once.find("a.css").unwrap() < once.find("b.css").unwrap());
        assert_eq!(with_stylesheet_links(&once, "d/p.xhtml", &css, false), once);
        let replaced = with_stylesheet_links(src, "d/p.xhtml", &css, true);
        assert!(!replaced.contains("old.css"));
        assert!(replaced.contains("href=\"a.css\""));
        assert_eq!(
            with_stylesheet_links("<p/>", "x.xhtml", &css, false),
            "<p/>"
        );
    }

    #[test]
    fn quick_open_ranks_prefix_then_substring_then_scattered_letters() {
        let files: Vec<String> = [
            "OEBPS/text/ch10.xhtml",
            "OEBPS/text/ch02.xhtml",
            "OEBPS/text/cover.xhtml",
            "OEBPS/styles/chapter.css",
            "OEBPS/images/fig2-1.png",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(quick_open("", &files), files);
        let r = quick_open("ch", &files);
        assert_eq!(
            &r[..3],
            &[
                "OEBPS/text/ch02.xhtml",
                "OEBPS/text/ch10.xhtml",
                "OEBPS/styles/chapter.css"
            ]
        );
        assert_eq!(quick_open("CH02", &files), vec!["OEBPS/text/ch02.xhtml"]);
        assert_eq!(quick_open("cvr", &files), vec!["OEBPS/text/cover.xhtml"]);
        assert_eq!(quick_open("fig", &files)[0], "OEBPS/images/fig2-1.png");
        assert_eq!(
            quick_open("styles", &files),
            vec!["OEBPS/styles/chapter.css"]
        );
        assert!(quick_open("zzz", &files).is_empty());
    }

    #[test]
    fn exporting_writes_files_and_numbers_clashes() {
        let b = book_with_images();
        let dir = std::env::temp_dir().join(format!("bookx-export-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let paths = vec![
            "OEBPS/images/a.png".to_string(),
            "OEBPS/images/a.png".to_string(),
        ];
        let out = export_files(&b, &paths, &dir).unwrap();
        assert_eq!(out.len(), 2);
        assert!(dir.join("a.png").exists() && dir.join("a-2.png").exists());
        assert!(export_files(&b, &["missing".to_string()], &dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
