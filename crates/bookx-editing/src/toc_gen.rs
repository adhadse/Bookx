//! Generating a table of contents (CE-121, CE-122, CE-123): from headings (levels 1..6), from the links of a
//! page (an inline contents page) or one entry per file; replacing or appending to the current contents.

use bookx_core::book::OpenBook;
use bookx_core::doc::{BlockKind, Span};
use bookx_core::toc::FlatToc;
use bookx_core::util::{file_name, is_external, parent_dir, resolve_href};

use crate::markup::{plain_text, scan, tag_attr, Event};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocSource {
    Headings,
    Links,
    Files,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TocMode {
    Replace,
    Append,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TocOptions {
    pub source: TocSource,
    /// Deepest heading level taken, 1..=6.
    pub max_level: u8,
    pub mode: TocMode,
    /// The page whose links form the contents ([`TocSource::Links`]).
    pub links_file: Option<String>,
}

impl Default for TocOptions {
    fn default() -> Self {
        TocOptions {
            source: TocSource::Headings,
            max_level: 3,
            mode: TocMode::Replace,
            links_file: None,
        }
    }
}

fn squash(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn headings(book: &OpenBook, max_level: u8) -> Vec<FlatToc> {
    let mut out: Vec<FlatToc> = Vec::new();
    for p in book.spine_paths() {
        let Some(doc) = book.chapter_doc(&p) else {
            continue;
        };
        let mut prev_depth = 0usize;
        let mut any = false;
        for b in &doc.blocks {
            let BlockKind::Heading(n) = b.kind else {
                continue;
            };
            if n > max_level {
                continue;
            }
            let title = squash(&b.plain_text());
            if title.is_empty() {
                continue;
            }
            let anchor = b.spans.iter().find_map(|s| match s {
                Span::Anchor(a) => Some(a.clone()),
                _ => None,
            });
            let mut depth = (n as usize).saturating_sub(1);
            if !any && out.is_empty() {
                depth = 0;
            }
            depth = depth.min(prev_depth + 1);
            let href = match anchor {
                Some(a) if any => format!("{p}#{a}"),
                _ => p.clone(),
            };
            out.push(FlatToc { depth, title, href });
            prev_depth = depth;
            any = true;
        }
    }
    out
}

fn files(book: &OpenBook) -> Vec<FlatToc> {
    book.spine_paths()
        .into_iter()
        .map(|p| {
            let title = book
                .chapter_doc(&p)
                .and_then(|d| {
                    d.blocks.iter().find_map(|b| match b.kind {
                        BlockKind::Heading(_) => {
                            Some(squash(&b.plain_text())).filter(|t| !t.is_empty())
                        }
                        _ => None,
                    })
                })
                .unwrap_or_else(|| {
                    let n = file_name(&p);
                    n.rsplit_once('.').map(|s| s.0).unwrap_or(n).to_string()
                });
            FlatToc {
                depth: 0,
                title,
                href: p,
            }
        })
        .collect()
}

/// Entries for the local links of `page` that point at a file of the book.
pub fn links_of(book: &OpenBook, page: &str) -> Vec<FlatToc> {
    let Some(src) = book.archive.get_string(page) else {
        return Vec::new();
    };
    let events = scan(&src);
    let mut out = Vec::new();
    for (i, e) in events.iter().enumerate() {
        let Event::Open { name, range, .. } = e else {
            continue;
        };
        if name != "a" {
            continue;
        }
        let Some(href) = tag_attr(&src[range.clone()], "href") else {
            continue;
        };
        if href.is_empty() || is_external(&href) {
            continue;
        }
        let (path, frag) = resolve_href(parent_dir(page), &href);
        let path = if path.is_empty() {
            page.to_string()
        } else {
            path
        };
        if !book.archive.contains(&path) {
            continue;
        }
        let mut text = String::new();
        for e in &events[i + 1..] {
            match e {
                Event::Close { name, .. } if name == "a" => break,
                Event::Text(r) => text.push_str(&src[r.clone()]),
                _ => {}
            }
        }
        let title = squash(&plain_text(&text));
        if title.is_empty() {
            continue;
        }
        let href = match frag {
            Some(f) => format!("{path}#{f}"),
            None => path,
        };
        out.push(FlatToc {
            depth: 0,
            title,
            href,
        });
    }
    out
}

/// Byte offset of the element carrying `id="frag"` (or `<a name="frag">`) in a page.
pub fn anchor_offset(src: &str, frag: &str) -> Option<usize> {
    scan(src).into_iter().find_map(|e| match e {
        Event::Open { name, range, .. } => {
            let tag = &src[range.clone()];
            let hit = tag_attr(tag, "id").as_deref() == Some(frag)
                || (name == "a" && tag_attr(tag, "name").as_deref() == Some(frag));
            hit.then_some(range.start)
        }
        _ => None,
    })
}

/// Builds the entries for the chosen source.
pub fn generate(book: &OpenBook, opts: &TocOptions) -> Vec<FlatToc> {
    match opts.source {
        TocSource::Headings => headings(book, opts.max_level.clamp(1, 6)),
        TocSource::Files => files(book),
        TocSource::Links => opts
            .links_file
            .as_deref()
            .map(|p| links_of(book, p))
            .unwrap_or_default(),
    }
}

/// Replaces the contents or appends the generated entries that are not in it yet (same target).
pub fn combine(existing: &[FlatToc], generated: Vec<FlatToc>, mode: TocMode) -> Vec<FlatToc> {
    match mode {
        TocMode::Replace => generated,
        TocMode::Append => {
            let mut out = existing.to_vec();
            for g in generated {
                if !out.iter().any(|e| e.href == g.href) {
                    out.push(g);
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::make_book;

    fn book() -> OpenBook {
        make_book(&[
            "<h1>Part One</h1><h2>First</h2><p>x</p><h4>Deep</h4>",
            "<h1>Part Two</h1><p>y</p>",
        ])
    }

    fn titles(v: &[FlatToc]) -> Vec<(usize, String)> {
        v.iter().map(|t| (t.depth, t.title.clone())).collect()
    }

    #[test]
    fn heading_levels_limit_the_depth() {
        let b = book();
        let one = generate(
            &b,
            &TocOptions {
                max_level: 1,
                ..Default::default()
            },
        );
        assert_eq!(
            titles(&one),
            vec![(0, "Part One".into()), (0, "Part Two".into())]
        );
        let three = generate(&b, &TocOptions::default());
        assert_eq!(
            titles(&three),
            vec![
                (0, "Part One".into()),
                (1, "First".into()),
                (0, "Part Two".into())
            ]
        );
        let six = generate(
            &b,
            &TocOptions {
                max_level: 6,
                ..Default::default()
            },
        );
        assert!(six.iter().any(|t| t.title == "Deep"));
        assert!(six.iter().all(|t| t.depth <= 3));
    }

    #[test]
    fn files_give_one_entry_per_spine_item() {
        let b = book();
        let f = generate(
            &b,
            &TocOptions {
                source: TocSource::Files,
                ..Default::default()
            },
        );
        assert_eq!(f.len(), 2);
        assert_eq!(f[1].title, "Part Two");
        assert!(f[0].href.ends_with("chapter-1.xhtml"));
    }

    #[test]
    fn links_of_a_page_become_entries_and_external_links_are_skipped() {
        let mut b = book();
        b.archive.set_string(
            "OEBPS/contents.xhtml",
            "<html><body><a href=\"chapter-1.xhtml\">One</a> <a href=\"chapter-2.xhtml#x\">Two\n  b</a><a href=\"http://x.org\">web</a><a href=\"missing.xhtml\">gone</a><a href=\"chapter-1.xhtml\"></a></body></html>",
        );
        let l = generate(
            &b,
            &TocOptions {
                source: TocSource::Links,
                links_file: Some("OEBPS/contents.xhtml".into()),
                ..Default::default()
            },
        );
        assert_eq!(titles(&l), vec![(0, "One".into()), (0, "Two b".into())]);
        assert_eq!(l[1].href, "OEBPS/chapter-2.xhtml#x");
        assert!(generate(
            &b,
            &TocOptions {
                source: TocSource::Links,
                ..Default::default()
            }
        )
        .is_empty());
    }

    #[test]
    fn anchors_are_found_by_id_or_name() {
        let src = "<p>x</p><h2 id=\"sec\">S</h2><a name=\"old\"/><p id='q'/>";
        assert_eq!(anchor_offset(src, "sec"), Some(8));
        assert_eq!(anchor_offset(src, "old"), src.find("<a name"));
        assert_eq!(anchor_offset(src, "q"), src.find("<p id"));
        assert_eq!(anchor_offset(src, "nope"), None);
    }

    #[test]
    fn append_keeps_existing_entries_and_adds_only_new_targets() {
        let b = book();
        let existing = vec![FlatToc {
            depth: 0,
            title: "Mine".into(),
            href: "OEBPS/chapter-1.xhtml".into(),
        }];
        let r#gen = generate(&b, &TocOptions::default());
        let out = combine(&existing, r#gen.clone(), TocMode::Append);
        assert_eq!(out[0].title, "Mine");
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].title, "Part Two");
        assert_eq!(
            combine(&existing, r#gen.clone(), TocMode::Replace).len(),
            r#gen.len()
        );
    }
}
