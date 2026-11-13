//! Where a library keeps its files: the layout choice (roadmap PP-C-3 / PP-C-7).
//!
//! * [`Layout::Tree`]: `Author/Title (id)/Title - Author.epub` and `cover.jpg` beside it (Calibre-like).
//! * [`Layout::Flat`]: `Author - Title.epub` straight in the library folder.
//! * [`Layout::Template`]: a path from the template language, for example `{author}/{series}/{title}`.
//! * [`Layout::Linked`]: books stay where the user keeps them; only the catalogue knows them, and
//!   Bookx never moves or deletes those files.
//!
//! Covers of the non-tree layouts live in `.covers/<id>.<ext>`. The planning here is pure: it only
//! computes relative paths, the library applies them.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

use crate::opf::Metadata;
use crate::util::sanitize_filename;

/// Folder for covers of the layouts that have no folder per book.
pub const COVERS_DIR: &str = ".covers";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Layout {
    #[default]
    Tree,
    Flat,
    Template(String),
    Linked,
}

/// Renders a template into relative path components (`ext` without dot is added to the last one).
pub type TemplateFn = fn(&str, &Metadata, &str) -> Result<Vec<String>, String>;

static RENDERER: OnceLock<TemplateFn> = OnceLock::new();

/// Registers the template renderer for the whole process (once; later calls are ignored). The app
/// and the command line pass `bookx_sources::template::render_library_path`.
pub fn register_template_renderer(f: TemplateFn) {
    let _ = RENDERER.set(f);
}

/// The renderer registered with [`register_template_renderer`].
pub fn global_renderer() -> Option<TemplateFn> {
    RENDERER.get().copied()
}

/// Where a library remembers its layout (`<root>/.bookx/layout.json`), so the app and the
/// command line always agree.
pub fn layout_path(root: &Path) -> PathBuf {
    root.join(crate::history::STATE_DIR).join("layout.json")
}

/// The layout saved for the library in `root`; the default tree when none was saved or the file is
/// damaged.
pub fn load_layout(root: &Path) -> Layout {
    std::fs::read(layout_path(root))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

pub fn save_layout(root: &Path, layout: &Layout) -> std::io::Result<()> {
    let p = layout_path(root);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = p.with_extension("json.tmp");
    std::fs::write(
        &tmp,
        serde_json::to_vec_pretty(layout).map_err(std::io::Error::other)?,
    )?;
    std::fs::rename(&tmp, &p)
}

/// Template of the "author / title" layout offered as a starting point.
pub const DEFAULT_TEMPLATE: &str = "{author}/{title}";

impl Layout {
    pub fn label(&self) -> &'static str {
        match self {
            Layout::Tree => "Author / Title folders",
            Layout::Flat => "One flat folder",
            Layout::Template(_) => "Custom template",
            Layout::Linked => "Link in place (do not copy)",
        }
    }

    /// Whether books are copied into and moved inside the library folder.
    pub fn manages_files(&self) -> bool {
        !matches!(self, Layout::Linked)
    }

    /// Short name for the command line.
    pub fn parse(name: &str, template: &str) -> Option<Layout> {
        match name.trim().to_ascii_lowercase().as_str() {
            "tree" | "author" => Some(Layout::Tree),
            "flat" => Some(Layout::Flat),
            "linked" | "link" => Some(Layout::Linked),
            "template" => Some(Layout::Template(if template.trim().is_empty() {
                DEFAULT_TEMPLATE.into()
            } else {
                template.into()
            })),
            _ => None,
        }
    }
}

/// Where a book goes, relative to the library folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    /// The book file.
    pub file: String,
    /// The folder that holds only this book (Tree layout), otherwise `None`.
    pub book_dir: Option<String>,
}

fn first_author(meta: &Metadata) -> String {
    sanitize_filename(
        meta.authors
            .first()
            .map(|s| s.as_str())
            .unwrap_or("Unknown"),
    )
}

/// Extension of a path without dot, lower case (`epub` when there is none).
pub fn ext_of(path: &str) -> String {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_else(|| "epub".into())
}

/// Relative path of the cover of a book that lives in `book_dir` (or in no folder of its own).
pub fn cover_rel(book_dir: Option<&str>, id: u64, ext: &str) -> String {
    match book_dir {
        Some(d) => format!("{d}/cover.{ext}"),
        None => format!("{COVERS_DIR}/{id}.{ext}"),
    }
}

/// Plans the place of a book. `taken` says which relative paths are in use by other books; on a
/// clash the id is added to the file name so two books never share a path. `Linked` has no plan
/// (the file stays where it is) and gives `None`.
pub fn plan(
    layout: &Layout,
    meta: &Metadata,
    id: u64,
    ext: &str,
    render: Option<TemplateFn>,
    taken: &dyn Fn(&str) -> bool,
) -> Option<Place> {
    let title = sanitize_filename(&meta.title);
    let author = first_author(meta);
    match layout {
        Layout::Linked => None,
        Layout::Tree => {
            let dir = format!("{author}/{title} ({id})");
            Some(Place {
                file: format!("{dir}/{title} - {author}.{ext}"),
                book_dir: Some(dir),
            })
        }
        Layout::Flat => {
            let plain = format!("{author} - {title}.{ext}");
            let file = if taken(&plain) {
                format!("{author} - {title} ({id}).{ext}")
            } else {
                plain
            };
            Some(Place {
                file,
                book_dir: None,
            })
        }
        Layout::Template(t) => {
            let comps = render
                .and_then(|f| f(t, meta, ext).ok())
                .filter(|c| !c.is_empty())
                .unwrap_or_else(|| vec![format!("{author} - {title}.{ext}")]);
            let comps: Vec<String> = comps
                .into_iter()
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty() && c != "." && c != "..")
                .collect();
            let comps = if comps.is_empty() {
                vec![format!("{author} - {title}.{ext}")]
            } else {
                comps
            };
            let plain = comps.join("/");
            let file = if taken(&plain) {
                let mut c = comps.clone();
                let last = c.pop().unwrap_or_default();
                let with_id = match last.rsplit_once('.') {
                    Some((stem, e)) => format!("{stem} ({id}).{e}"),
                    None => format!("{last} ({id})"),
                };
                c.push(with_id);
                c.join("/")
            } else {
                plain
            };
            Some(Place {
                file,
                book_dir: None,
            })
        }
    }
}

/// Checks a template before it is saved: parses and renders it for a sample book.
pub fn check_template(src: &str, render: TemplateFn) -> Result<String, String> {
    let sample = Metadata {
        title: "Sample Title".into(),
        authors: vec!["Jane Author".into()],
        series: "Sample Series".into(),
        series_index: 2.0,
        ..Default::default()
    };
    let comps = render(src, &sample, "epub")?;
    if comps.is_empty() {
        return Err("the template produces an empty path".into());
    }
    Ok(comps.join("/"))
}

/// Paths in use, as a lookup for [`plan`].
pub fn taken_set<'a>(files: impl Iterator<Item = &'a str>) -> HashSet<String> {
    files.map(str::to_string).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(title: &str, author: &str) -> Metadata {
        Metadata {
            title: title.into(),
            authors: vec![author.into()],
            ..Default::default()
        }
    }

    fn render(src: &str, m: &Metadata, ext: &str) -> Result<Vec<String>, String> {
        let s = src.replace("{title}", &m.title).replace(
            "{author}",
            m.authors.first().map(String::as_str).unwrap_or("x"),
        );
        let mut comps: Vec<String> = s.split('/').map(sanitize_filename).collect();
        if let Some(last) = comps.last_mut() {
            last.push_str(&format!(".{ext}"));
        }
        Ok(comps)
    }

    fn free(_: &str) -> bool {
        false
    }

    #[test]
    fn tree_keeps_the_calibre_like_shape() {
        let p = plan(
            &Layout::Tree,
            &meta("Dune", "Frank Herbert"),
            7,
            "epub",
            None,
            &free,
        )
        .unwrap();
        assert_eq!(p.file, "Frank Herbert/Dune (7)/Dune - Frank Herbert.epub");
        assert_eq!(p.book_dir.as_deref(), Some("Frank Herbert/Dune (7)"));
        assert_eq!(
            cover_rel(p.book_dir.as_deref(), 7, "jpg"),
            "Frank Herbert/Dune (7)/cover.jpg"
        );
    }

    #[test]
    fn flat_adds_the_id_only_on_a_clash() {
        let m = meta("Dune", "Frank Herbert");
        let p = plan(&Layout::Flat, &m, 7, "epub", None, &free).unwrap();
        assert_eq!(p.file, "Frank Herbert - Dune.epub");
        assert!(p.book_dir.is_none());
        let p = plan(&Layout::Flat, &m, 7, "epub", None, &|f| {
            f == "Frank Herbert - Dune.epub"
        })
        .unwrap();
        assert_eq!(p.file, "Frank Herbert - Dune (7).epub");
        assert_eq!(cover_rel(None, 7, "png"), ".covers/7.png");
    }

    #[test]
    fn template_paths_are_safe_and_unique() {
        let l = Layout::Template("{author}/{title}".into());
        let m = meta("A-B: C", "Ann \"Q\" Lee");
        let p = plan(&l, &m, 3, "epub", Some(render), &free).unwrap();
        assert!(!p.file.contains(".."));
        assert_eq!(p.file.matches('/').count(), 1, "{}", p.file);
        let q = plan(&l, &m, 3, "epub", Some(render), &|f| f == p.file).unwrap();
        assert!(q.file.ends_with("(3).epub"), "{}", q.file);
    }

    #[test]
    fn a_broken_or_missing_template_falls_back_to_a_flat_name() {
        let l = Layout::Template("{nope".into());
        let bad: TemplateFn = |_, _, _| Err("bad".into());
        let p = plan(&l, &meta("T", "A"), 1, "epub", Some(bad), &free).unwrap();
        assert_eq!(p.file, "A - T.epub");
        let p = plan(&l, &meta("T", "A"), 1, "epub", None, &free).unwrap();
        assert_eq!(p.file, "A - T.epub");
    }

    #[test]
    fn linked_has_no_plan_and_does_not_manage_files() {
        assert!(plan(&Layout::Linked, &meta("T", "A"), 1, "epub", None, &free).is_none());
        assert!(!Layout::Linked.manages_files());
        assert!(Layout::Flat.manages_files());
    }

    #[test]
    fn unicode_and_long_names_stay_valid() {
        let long = "ä".repeat(300);
        let p = plan(
            &Layout::Flat,
            &meta(&long, "Łukasz 山田"),
            9,
            "epub",
            None,
            &free,
        )
        .unwrap();
        assert!(p.file.chars().count() < 200);
        assert!(!p.file.contains('/'));
        let p = plan(&Layout::Tree, &meta("", ""), 2, "epub", None, &free).unwrap();
        assert!(p.file.starts_with("Unknown/"));
    }

    #[test]
    fn the_layout_is_remembered_per_library() {
        let d = std::env::temp_dir().join(format!("bookx-layout-{}", uuid::Uuid::new_v4()));
        assert_eq!(load_layout(&d), Layout::Tree);
        save_layout(&d, &Layout::Template("{author}/{title}".into())).unwrap();
        assert_eq!(load_layout(&d), Layout::Template("{author}/{title}".into()));
        std::fs::write(layout_path(&d), "nonsense").unwrap();
        assert_eq!(
            load_layout(&d),
            Layout::Tree,
            "a damaged file falls back to the default"
        );
        let _ = std::fs::remove_dir_all(d);
    }

    #[test]
    fn layout_names_parse() {
        assert_eq!(Layout::parse("flat", ""), Some(Layout::Flat));
        assert_eq!(Layout::parse("Tree", ""), Some(Layout::Tree));
        assert_eq!(Layout::parse("link", ""), Some(Layout::Linked));
        assert_eq!(
            Layout::parse("template", ""),
            Some(Layout::Template(DEFAULT_TEMPLATE.into()))
        );
        assert_eq!(Layout::parse("zig", ""), None);
    }

    #[test]
    fn template_check_reports_errors_and_a_sample() {
        assert_eq!(
            check_template("{author}/{title}", render).unwrap(),
            "Jane Author/Sample Title.epub"
        );
        let bad: TemplateFn = |_, _, _| Err("unclosed".into());
        assert_eq!(check_template("{", bad).unwrap_err(), "unclosed");
    }
}
