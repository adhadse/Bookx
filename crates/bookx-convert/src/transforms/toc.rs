//! Build a table of contents from headings (C13).

use std::collections::HashSet;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use bookx_core::toc::{TocEntry, write_toc};
use bookx_core::util::join_path;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::markup::{escape_attr, get_attr, plain_text, set_attr};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutoTocMode {
    /// Only when the book has no table of contents.
    #[default]
    IfMissing,
    /// Replace any existing table of contents.
    Force,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AutoToc {
    pub mode: AutoTocMode,
    /// Deepest heading level included (1..=3).
    pub max_level: u8,
}

impl Default for AutoToc {
    fn default() -> Self {
        AutoToc {
            mode: AutoTocMode::IfMissing,
            max_level: 3,
        }
    }
}

struct Heading {
    level: u8,
    title: String,
    href: String,
}

fn build_tree(items: &[Heading]) -> Vec<TocEntry> {
    // stack of (level, path-into-tree)
    let mut roots: Vec<TocEntry> = Vec::new();
    let mut stack: Vec<u8> = Vec::new();
    for h in items {
        while stack.last().is_some_and(|l| *l >= h.level) {
            stack.pop();
        }
        let entry = TocEntry {
            title: h.title.clone(),
            href: h.href.clone(),
            children: vec![],
        };
        let mut level_nodes = &mut roots;
        for _ in 0..stack.len() {
            level_nodes = &mut level_nodes.last_mut().expect("parent exists").children;
        }
        level_nodes.push(entry);
        stack.push(h.level);
    }
    roots
}

fn used_ids(html: &str) -> HashSet<String> {
    Regex::new(r#"(?i)\sid\s*=\s*["']([^"']*)["']"#)
        .expect("static regex")
        .captures_iter(html)
        .map(|c| c[1].to_string())
        .collect()
}

impl AutoToc {
    /// Adds ids to headings and returns the entries found, plus the new text of the file if ids
    /// were added.
    fn scan(&self, path: &str, html: &str, counter: &mut usize) -> (Vec<Heading>, Option<String>) {
        let re = Regex::new(r"(?is)<h([1-3])(\s[^>]*)?>(.*?)</h[1-3]\s*>").expect("static regex");
        let mut ids = used_ids(html);
        let mut found = Vec::new();
        let mut changed = false;
        let out = re.replace_all(html, |c: &regex::Captures| {
            let level: u8 = c[1].parse().unwrap_or(1);
            let whole = c[0].to_string();
            if level > self.max_level {
                return whole;
            }
            let title = plain_text(&c[3])
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ");
            if title.is_empty() {
                return whole;
            }
            let open_end = whole.find('>').map(|p| p + 1).unwrap_or(0);
            let open = &whole[..open_end];
            let (open, id) = match get_attr(open, "id") {
                Some(id) if !id.is_empty() => (open.to_string(), id),
                _ => {
                    let id = loop {
                        *counter += 1;
                        let id = format!("bookx-toc-{counter}");
                        if !ids.contains(&id) {
                            break id;
                        }
                    };
                    ids.insert(id.clone());
                    changed = true;
                    (set_attr(open, "id", &escape_attr(&id)), id)
                }
            };
            found.push(Heading {
                level,
                title,
                href: format!("{path}#{id}"),
            });
            format!("{}{}", open, &whole[open_end..])
        });
        let new = changed.then(|| out.into_owned());
        (found, new)
    }
}

fn ensure_toc_document(book: &mut OpenBook) -> Result<()> {
    if book.package.nav_item().is_some() || book.package.ncx_item().is_some() {
        return Ok(());
    }
    let path = join_path(book.package.base_dir(), "nav.xhtml");
    book.archive.set_string(
        &path,
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\">\n<head><title>Contents</title></head>\n<body>\n  <nav epub:type=\"toc\" id=\"toc\">\n    <h1>Contents</h1>\n    <ol>\n    </ol>\n  </nav>\n</body>\n</html>\n",
    );
    let pkg = book.package.clone();
    pkg.add_manifest_item(
        &mut book.archive,
        "bookx-nav",
        &path,
        "application/xhtml+xml",
        "nav",
    )?;
    book.reload_package()
}

impl Transform for AutoToc {
    fn id(&self) -> &'static str {
        "auto_toc"
    }

    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        if self.mode == AutoTocMode::IfMissing && !book.toc().is_empty() {
            return Ok(TransformReport {
                files_changed: 0,
                notes: vec!["the book already has a table of contents".into()],
            });
        }
        let mut report = TransformReport::default();
        let mut counter = 0usize;
        let mut headings = Vec::new();
        for path in book.spine_paths() {
            let Some(text) = book.archive.get_string(&path) else {
                continue;
            };
            let (found, new) = self.scan(&path, &text, &mut counter);
            headings.extend(found);
            if let Some(new) = new {
                book.archive.set_string(&path, &new);
                report.files_changed += 1;
            }
        }
        if headings.is_empty() {
            report.notes.push("no headings found".into());
            return Ok(report);
        }
        let tree = build_tree(&headings);
        let before = book.toc();
        if before == tree {
            return Ok(report);
        }
        ensure_toc_document(book)?;
        let pkg = book.package.clone();
        write_toc(&mut book.archive, &pkg, &tree)?;
        report.files_changed += 1;
        report.notes.push(format!("{} entries", headings.len()));
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use bookx_core::toc::flatten;

    fn without_toc(chapters: &[(&str, &str)]) -> OpenBook {
        let mut b = book_with(chapters);
        let pkg = b.package.clone();
        write_toc(&mut b.archive, &pkg, &[]).unwrap();
        assert!(b.toc().is_empty());
        b
    }

    const DOC: &[(&str, &str)] = &[
        (
            "one",
            "<h1>Part One</h1><p>x</p><h2>Section <i>A</i></h2><p>y</p><h3>Sub</h3><h4>Too deep</h4>",
        ),
        (
            "two",
            "<h1 id=\"keep\">Part Two</h1><h2>  </h2><p>z</p><h2>Another\n  Section</h2>",
        ),
    ];

    #[test]
    fn builds_nested_toc_when_missing() {
        let mut b = without_toc(DOC);
        let r = AutoToc::default().apply(&mut b).unwrap();
        assert!(r.files_changed >= 2);
        let again = OpenBook::from_archive(
            bookx_core::archive::Archive::from_bytes(b.archive.to_bytes().unwrap()).unwrap(),
        )
        .unwrap();
        let toc = again.toc();
        assert_eq!(toc.len(), 2);
        assert_eq!(toc[0].title, "Part One");
        assert_eq!(toc[0].children.len(), 1);
        assert_eq!(toc[0].children[0].title, "Section A");
        assert_eq!(toc[0].children[0].children[0].title, "Sub");
        assert!(toc[0].children[0].children.len() == 1, "h4 is excluded");
        assert_eq!(toc[1].title, "Part Two");
        assert_eq!(toc[1].children[0].title, "Another Section");
        assert!(
            toc[1].href.ends_with("#keep"),
            "existing ids are reused: {}",
            toc[1].href
        );
    }

    #[test]
    fn links_point_at_real_ids() {
        let mut b = without_toc(DOC);
        AutoToc::default().apply(&mut b).unwrap();
        for e in flatten(&b.toc()) {
            let (path, frag) = e.href.split_once('#').unwrap();
            let html = b.archive.get_string(path).unwrap();
            assert!(
                html.contains(&format!("id=\"{frag}\"")),
                "{} missing in {path}",
                frag
            );
        }
    }

    #[test]
    fn existing_toc_is_kept_unless_forced() {
        let mut b = book_with(DOC);
        let snap = snapshot(&b);
        let r = AutoToc::default().apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 0);
        assert_eq!(snapshot(&b), snap);
        let r = AutoToc {
            mode: AutoTocMode::Force,
            max_level: 2,
        }
        .apply(&mut b)
        .unwrap();
        assert!(r.files_changed > 0);
        let flat = flatten(&b.toc());
        assert!(flat.iter().all(|f| f.depth <= 1));
        assert_eq!(flat.len(), 4);
    }

    #[test]
    fn idempotent_in_both_modes() {
        for mode in [AutoTocMode::IfMissing, AutoTocMode::Force] {
            let mut b = without_toc(DOC);
            let t = AutoToc { mode, max_level: 3 };
            t.apply(&mut b).unwrap();
            let snap = snapshot(&b);
            let r = t.apply(&mut b).unwrap();
            assert_eq!(r.files_changed, 0, "{mode:?}");
            assert_eq!(snapshot(&b), snap, "{mode:?}");
        }
    }

    #[test]
    fn generated_ids_avoid_collisions() {
        let mut b = without_toc(&[("a", "<h1 id=\"bookx-toc-1\">Taken</h1><h1>Free</h1>")]);
        AutoToc::default().apply(&mut b).unwrap();
        let html = chapter(&b, 0);
        assert_eq!(html.matches("id=\"bookx-toc-1\"").count(), 1);
        assert!(html.contains("bookx-toc-2"));
    }

    #[test]
    fn no_headings_changes_nothing() {
        let mut b = without_toc(&[("a", "<p>just text</p>")]);
        let snap = snapshot(&b);
        let r = AutoToc::default().apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 0);
        assert!(r.notes.iter().any(|n| n.contains("no headings")));
        assert_eq!(snapshot(&b), snap);
        assert_eq!(AutoToc::default().id(), "auto_toc");
    }

    #[test]
    fn creates_nav_document_when_book_has_none() {
        let mut b = without_toc(&[("a", "<h1>Hi</h1>")]);
        // strip both navigation documents
        let nav = b.package.nav_item().unwrap().id.clone();
        let ncx = b.package.ncx_item().unwrap().id.clone();
        let pkg = b.package.clone();
        for id in [nav, ncx] {
            let path = pkg.item_path(pkg.item_by_id(&id).unwrap());
            pkg.remove_manifest_item(&mut b.archive, &id).unwrap();
            b.archive.remove(&path);
        }
        b.reload_package().unwrap();
        assert!(b.package.nav_item().is_none() && b.package.ncx_item().is_none());
        AutoToc::default().apply(&mut b).unwrap();
        assert!(b.package.nav_item().is_some());
        assert_eq!(b.toc()[0].title, "Hi");
    }

    #[test]
    fn tree_builder_handles_skipped_levels() {
        let items = vec![
            Heading {
                level: 1,
                title: "a".into(),
                href: "a".into(),
            },
            Heading {
                level: 3,
                title: "b".into(),
                href: "b".into(),
            },
            Heading {
                level: 2,
                title: "c".into(),
                href: "c".into(),
            },
            Heading {
                level: 1,
                title: "d".into(),
                href: "d".into(),
            },
        ];
        let t = build_tree(&items);
        assert_eq!(t.len(), 2);
        assert_eq!(t[0].children.len(), 2);
        assert_eq!(t[0].children[0].title, "b");
        assert_eq!(t[0].children[1].title, "c");
    }
}
