//! Split oversized XHTML files at block boundaries (Kindle and old readers choke on big files).

use std::collections::{HashMap, HashSet};

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Transform, TransformReport};
use bookx_core::opf::SpineItem;
use bookx_core::util::{extension, file_name, join_path, parent_dir, relative_href, resolve_href};
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::markup::{Tok, body_range, is_void, remove_attr, tag_info, tokenize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SplitLargeHtml {
    /// Files larger than this many bytes are split.
    pub max_bytes: usize,
}

impl Default for SplitLargeHtml {
    fn default() -> Self {
        SplitLargeHtml { max_bytes: 260_000 }
    }
}

const CONTAINERS: &[&str] = &[
    "div",
    "section",
    "article",
    "blockquote",
    "main",
    "aside",
    "body",
];

struct Cut {
    /// Offset in the body content (just after the closing tag).
    offset: usize,
    /// Open container tags at this point, outermost first.
    open: Vec<String>,
    names: Vec<String>,
}

fn cut_candidates(body: &str) -> Vec<Cut> {
    let mut cuts = Vec::new();
    let mut stack: Vec<(String, String)> = Vec::new();
    let mut pos = 0;
    for t in tokenize(body) {
        let len = t.as_str().len();
        pos += len;
        let Tok::Tag(tag) = t else { continue };
        let Some(info) = tag_info(tag) else { continue };
        if info.closing {
            if let Some(i) = stack.iter().rposition(|(n, _)| *n == info.name) {
                stack.truncate(i);
                if stack.iter().all(|(n, _)| CONTAINERS.contains(&n.as_str())) {
                    cuts.push(Cut {
                        offset: pos,
                        open: stack.iter().map(|(_, o)| o.clone()).collect(),
                        names: stack.iter().map(|(n, _)| n.clone()).collect(),
                    });
                }
            }
        } else if info.self_closing || is_void(&info.name) {
            if info.name == "hr" && stack.iter().all(|(n, _)| CONTAINERS.contains(&n.as_str())) {
                cuts.push(Cut {
                    offset: pos,
                    open: stack.iter().map(|(_, o)| o.clone()).collect(),
                    names: stack.iter().map(|(n, _)| n.clone()).collect(),
                });
            }
        } else {
            stack.push((info.name, tag.to_string()));
        }
    }
    cuts
}

struct Part {
    text: String,
}

fn split_document(text: &str, max: usize) -> Option<Vec<Part>> {
    let (bs, be) = body_range(text)?;
    let head = &text[..bs];
    let tail = &text[be..];
    let body = &text[bs..be];
    let cuts = cut_candidates(body);
    if cuts.is_empty() {
        return None;
    }
    let reopen = |c: Option<&Cut>| -> String {
        c.map(|c| {
            c.open
                .iter()
                .map(|o| remove_attr(o, "id"))
                .collect::<String>()
        })
        .unwrap_or_default()
    };
    let close = |c: Option<&Cut>| -> String {
        c.map(|c| {
            c.names
                .iter()
                .rev()
                .map(|n| format!("</{n}>"))
                .collect::<String>()
        })
        .unwrap_or_default()
    };
    let size = |start: usize, end: usize, from: Option<&Cut>, to: Option<&Cut>| {
        head.len() + tail.len() + reopen(from).len() + (end - start) + close(to).len()
    };

    let mut parts = Vec::new();
    let mut start = 0usize;
    let mut from: Option<&Cut> = None;
    loop {
        if size(start, body.len(), from, None) <= max {
            break;
        }
        let next = cuts
            .iter()
            .rfind(|c| {
                c.offset > start
                    && c.offset < body.len()
                    && size(start, c.offset, from, Some(c)) <= max
            })
            .or_else(|| {
                cuts.iter()
                    .find(|c| c.offset > start && c.offset < body.len())
            });
        let Some(cut) = next else { break };
        parts.push(format!(
            "{head}{}{}{}{tail}",
            reopen(from),
            &body[start..cut.offset],
            close(Some(cut))
        ));
        start = cut.offset;
        from = Some(cut);
    }
    if parts.is_empty() {
        return None;
    }
    parts.push(format!("{head}{}{}{tail}", reopen(from), &body[start..]));
    Some(parts.into_iter().map(|text| Part { text }).collect())
}

fn ids_in(html: &str) -> HashSet<String> {
    Regex::new(r#"(?i)\s(?:id|name)\s*=\s*["']([^"']*)["']"#)
        .expect("static regex")
        .captures_iter(html)
        .map(|c| c[1].to_string())
        .collect()
}

/// Rewrites `href`/`src` values of one document according to the `moved` map
/// (`(original path, fragment)` -> new path).
fn rewrite_refs(
    text: &str,
    doc_path: &str,
    doc_origin: &str,
    moved: &HashMap<(String, String), String>,
) -> String {
    let re =
        Regex::new(r#"(?i)(\s(?:href|src)\s*=\s*)("([^"]*)"|'([^']*)')"#).expect("static regex");
    let base = parent_dir(doc_origin).to_string();
    re.replace_all(text, |c: &regex::Captures| {
        let value = c.get(3).or(c.get(4)).map(|m| m.as_str()).unwrap_or("");
        if value.is_empty() || bookx_core::util::is_external(value) {
            return c[0].to_string();
        }
        let (path, frag) = resolve_href(&base, value);
        let Some(frag) = frag else {
            return c[0].to_string();
        };
        let target_orig = if path.is_empty() {
            doc_origin.to_string()
        } else {
            path
        };
        let new_target = moved
            .get(&(target_orig.clone(), frag.clone()))
            .cloned()
            .unwrap_or_else(|| target_orig.clone());
        // `doc_path` may differ from `doc_origin` (a part); compare against where we are now
        let here_is_target = new_target == doc_path;
        let new_value = if here_is_target && value.starts_with('#') {
            return c[0].to_string();
        } else if here_is_target {
            format!("#{frag}")
        } else {
            relative_href(doc_path, &format!("{new_target}#{frag}"))
        };
        if new_value == value {
            c[0].to_string()
        } else {
            format!("{}\"{}\"", &c[1], new_value)
        }
    })
    .into_owned()
}

impl Transform for SplitLargeHtml {
    fn id(&self) -> &'static str {
        "split_large_html"
    }

    fn apply(&self, book: &mut OpenBook) -> Result<TransformReport> {
        let mut report = TransformReport::default();
        let spine = book.spine_paths();
        // (original path) -> part paths in order, including the original as first
        let mut splits: Vec<(String, Vec<String>)> = Vec::new();
        let mut moved: HashMap<(String, String), String> = HashMap::new();
        let mut new_items: Vec<(String, String, String)> = Vec::new(); // (id, path, after-id)
        let mut taken: HashSet<String> = book.archive.names().iter().cloned().collect();
        let mut origin: HashMap<String, String> = HashMap::new();

        for path in &spine {
            let Some(text) = book.archive.get_string(path) else {
                continue;
            };
            if text.len() <= self.max_bytes {
                continue;
            }
            let Some(parts) = split_document(&text, self.max_bytes) else {
                report.notes.push(format!("{path}: no safe split point"));
                continue;
            };
            let stem = file_name(path)
                .rsplit_once('.')
                .map(|(s, _)| s)
                .unwrap_or(file_name(path));
            let ext = extension(path);
            let orig_id = book
                .package
                .item_by_path(path)
                .map(|i| i.id.clone())
                .unwrap_or_default();
            let mut names = vec![path.clone()];
            let mut part_ids: Vec<HashSet<String>> = vec![ids_in(&parts[0].text)];
            book.archive.set_string(path, &parts[0].text);
            for (k, part) in parts.iter().enumerate().skip(1) {
                let mut n = k;
                let new_path = loop {
                    let cand = join_path(parent_dir(path), &format!("{stem}_split_{n}.{ext}"));
                    if !taken.contains(&cand) {
                        break cand;
                    }
                    n += 1;
                };
                taken.insert(new_path.clone());
                book.archive.set_string(&new_path, &part.text);
                let id = format!("{orig_id}_split_{k}");
                new_items.push((id, new_path.clone(), orig_id.clone()));
                origin.insert(new_path.clone(), path.clone());
                names.push(new_path);
                part_ids.push(ids_in(&part.text));
            }
            // ids that now live in a later part
            // an id present in the first part wins (it is where the original anchor stayed)
            for (k, ids) in part_ids.iter().enumerate().skip(1) {
                for id in ids.iter().filter(|id| !part_ids[0].contains(*id)) {
                    moved
                        .entry((path.clone(), id.clone()))
                        .or_insert_with(|| names[k].clone());
                }
            }
            report.files_changed += 1;
            report.notes.push(format!("{path}: {} parts", names.len()));
            splits.push((path.clone(), names));
        }
        if splits.is_empty() {
            return Ok(report);
        }

        // manifest + spine
        let pkg = book.package.clone();
        let mut used_ids: HashSet<String> = pkg.manifest.iter().map(|i| i.id.clone()).collect();
        let mut final_items = Vec::new();
        for (id, path, after) in new_items {
            let mut id2 = id.clone();
            let mut n = 1;
            while used_ids.contains(&id2) {
                n += 1;
                id2 = format!("{id}_{n}");
            }
            used_ids.insert(id2.clone());
            pkg.add_manifest_item(&mut book.archive, &id2, &path, "application/xhtml+xml", "")?;
            final_items.push((id2, after));
        }
        let mut order: Vec<SpineItem> = Vec::new();
        for s in &pkg.spine {
            order.push(s.clone());
            for (id, after) in &final_items {
                if *after == s.idref {
                    order.push(SpineItem {
                        idref: id.clone(),
                        linear: s.linear,
                    });
                }
            }
        }
        pkg.set_spine(&mut book.archive, &order)?;
        book.reload_package()?;

        // links: spine documents, navigation document and NCX
        if !moved.is_empty() {
            let mut docs: Vec<String> = book.spine_paths();
            for it in [book.package.nav_item(), book.package.ncx_item()]
                .into_iter()
                .flatten()
            {
                docs.push(book.package.item_path(it));
            }
            docs.sort();
            docs.dedup();
            for doc in docs {
                let Some(text) = book.archive.get_string(&doc) else {
                    continue;
                };
                let doc_origin = origin.get(&doc).cloned().unwrap_or_else(|| doc.clone());
                let new = rewrite_refs(&text, &doc, &doc_origin, &moved);
                if new != text {
                    book.archive.set_string(&doc, &new);
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::super::testutil::*;
    use super::*;
    use bookx_core::toc::flatten;

    fn para(i: usize) -> String {
        format!(
            "<p id=\"p{i}\">Paragraph number {i} with some filler text to take up room in the file.</p>\n"
        )
    }

    fn big_chapter(n: usize) -> String {
        (0..n).map(para).collect()
    }

    #[test]
    fn splits_at_paragraph_boundaries_within_limit() {
        let mut b = book_with(&[("A", &big_chapter(60)), ("B", "<p>small</p>")]);
        let t = SplitLargeHtml { max_bytes: 1200 };
        let r = t.apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 1);
        let spine = b.spine_paths();
        assert!(spine.len() > 3, "{spine:?}");
        for p in &spine {
            let txt = b.archive.get_string(p).unwrap();
            assert!(txt.len() <= 1200, "{p} is {} bytes", txt.len());
            // every part is well-formed XML
            roxmltree::Document::parse_with_options(
                &txt,
                roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                },
            )
            .unwrap_or_else(|e| panic!("{p}: {e}\n{txt}"));
        }
        // order: parts directly follow the original, B stays last
        assert!(spine[0].ends_with("chapter-1.xhtml"));
        assert!(spine[1].contains("chapter-1_split_1"));
        assert!(spine.last().unwrap().ends_with("chapter-2.xhtml"));
    }

    #[test]
    fn no_paragraph_is_lost_or_split() {
        let mut b = book_with(&[("A", &big_chapter(60))]);
        SplitLargeHtml { max_bytes: 1500 }.apply(&mut b).unwrap();
        let mut seen = Vec::new();
        for p in b.spine_paths() {
            let txt = b.archive.get_string(&p).unwrap();
            let doc = roxmltree::Document::parse_with_options(
                &txt,
                roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                },
            )
            .unwrap();
            for n in doc.descendants().filter(|n| n.has_tag_name("p")) {
                seen.push(n.attribute("id").unwrap().to_string());
                assert!(n.text().unwrap().ends_with("in the file."));
            }
        }
        let want: Vec<String> = (0..60).map(|i| format!("p{i}")).collect();
        assert_eq!(seen, want);
    }

    #[test]
    fn nav_and_ncx_links_follow_moved_anchors() {
        let mut b = book_with(&[("A", &big_chapter(60))]);
        // add a TOC entry that targets an anchor deep in the file
        let pkg = b.package.clone();
        let target = format!("{}#p55", b.spine_paths()[0]);
        let toc = vec![
            bookx_core::toc::TocEntry {
                title: "start".into(),
                href: b.spine_paths()[0].clone(),
                children: vec![],
            },
            bookx_core::toc::TocEntry {
                title: "deep".into(),
                href: target,
                children: vec![],
            },
        ];
        bookx_core::toc::write_toc(&mut b.archive, &pkg, &toc).unwrap();
        SplitLargeHtml { max_bytes: 1500 }.apply(&mut b).unwrap();
        let flat = flatten(&b.toc());
        assert_eq!(flat.len(), 2);
        assert_eq!(flat[0].href, "OEBPS/chapter-1.xhtml");
        let (path, frag) = flat[1].href.split_once('#').unwrap();
        assert_eq!(frag, "p55");
        assert_ne!(
            path, "OEBPS/chapter-1.xhtml",
            "anchor moved to a later part"
        );
        assert!(b.archive.get_string(path).unwrap().contains("id=\"p55\""));
        assert!(b.spine_paths().contains(&path.to_string()));
    }

    #[test]
    fn in_text_links_to_moved_anchors_are_updated() {
        let mut body = String::from("<p><a href=\"#p50\">jump</a></p>\n");
        body.push_str(&big_chapter(60));
        body.push_str("<p><a href=\"#p0\">back</a> <a href=\"chapter-1.xhtml#p58\">self</a> <a href=\"https://x.org/#p58\">ext</a></p>");
        let mut b = book_with(&[
            ("A", &body),
            ("B", "<p><a href=\"chapter-1.xhtml#p59\">x</a></p>"),
        ]);
        SplitLargeHtml { max_bytes: 1500 }.apply(&mut b).unwrap();
        let spine = b.spine_paths();
        let first = b.archive.get_string(&spine[0]).unwrap();
        let link = Regex::new(r#"href="([^"]*)">jump"#)
            .unwrap()
            .captures(&first)
            .unwrap()[1]
            .to_string();
        assert!(
            link.contains("chapter-1_split_") && link.ends_with("#p50"),
            "{link}"
        );
        // the target file really holds the id
        let target_file = format!("OEBPS/{}", link.split('#').next().unwrap());
        assert!(
            b.archive
                .get_string(&target_file)
                .unwrap()
                .contains("id=\"p50\"")
        );
        let last_part = b.archive.get_string(&spine[spine.len() - 2]).unwrap();
        // back-link to the first part uses a relative file reference or stays a fragment
        assert!(last_part.contains(">back</a>"));
        let back = Regex::new(r#"href="([^"]*)">back"#)
            .unwrap()
            .captures(&last_part)
            .unwrap()[1]
            .to_string();
        assert!(back == "chapter-1.xhtml#p0", "{back}");
        assert!(last_part.contains("href=\"https://x.org/#p58\""));
        // chapter B pointing into the split file
        let bdoc = b.archive.get_string(spine.last().unwrap()).unwrap();
        assert!(bdoc.contains("chapter-1_split_"), "{bdoc}");
    }

    #[test]
    fn wrapper_divs_are_closed_and_reopened() {
        let inner: String = (0..40).map(para).collect();
        let body = format!("<div class=\"wrap\" id=\"outer\">{inner}</div>");
        let mut b = book_with(&[("A", &body)]);
        SplitLargeHtml { max_bytes: 1500 }.apply(&mut b).unwrap();
        let spine = b.spine_paths();
        assert!(spine.len() > 2);
        let mut ids = Vec::new();
        for p in &spine {
            let txt = b.archive.get_string(p).unwrap();
            let doc = roxmltree::Document::parse_with_options(
                &txt,
                roxmltree::ParsingOptions {
                    allow_dtd: true,
                    ..Default::default()
                },
            )
            .unwrap();
            let div = doc.descendants().find(|n| n.has_tag_name("div")).unwrap();
            assert_eq!(div.attribute("class"), Some("wrap"));
            for n in doc.descendants().filter(|n| n.attribute("id").is_some()) {
                ids.push(n.attribute("id").unwrap().to_string());
            }
        }
        let outer = ids.iter().filter(|i| *i == "outer").count();
        assert_eq!(outer, 1, "reopened wrappers must not repeat the id");
    }

    #[test]
    fn small_files_and_unsplittable_files_are_untouched() {
        let mut b = book_with(&[("A", "<p>tiny</p>")]);
        let snap = snapshot(&b);
        assert_eq!(
            SplitLargeHtml { max_bytes: 100_000 }
                .apply(&mut b)
                .unwrap()
                .files_changed,
            0
        );
        assert_eq!(snapshot(&b), snap);
        // one giant paragraph cannot be split
        let giant = format!("<p>{}</p>", "word ".repeat(1000));
        let mut g = book_with(&[("A", &giant)]);
        let r = SplitLargeHtml { max_bytes: 1000 }.apply(&mut g).unwrap();
        assert_eq!(r.files_changed, 0);
        assert!(r.notes.iter().any(|n| n.contains("no safe split point")));
        assert_eq!(g.spine_paths().len(), 1);
    }

    #[test]
    fn idempotent() {
        let mut b = book_with(&[("A", &big_chapter(60))]);
        let t = SplitLargeHtml { max_bytes: 1500 };
        t.apply(&mut b).unwrap();
        let snap = snapshot(&b);
        let r = t.apply(&mut b).unwrap();
        assert_eq!(r.files_changed, 0, "{r:?}");
        assert_eq!(snapshot(&b), snap);
        assert_eq!(t.id(), "split_large_html");
        assert_eq!(SplitLargeHtml::default().max_bytes, 260_000);
    }

    #[test]
    fn manifest_has_unique_ids_and_matches_spine() {
        let mut b = book_with(&[("A", &big_chapter(60))]);
        SplitLargeHtml { max_bytes: 1500 }.apply(&mut b).unwrap();
        let mut ids: Vec<&str> = b.package.manifest.iter().map(|i| i.id.as_str()).collect();
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n);
        for s in &b.package.spine {
            assert!(b.package.item_by_id(&s.idref).is_some());
        }
        for p in b.spine_paths() {
            assert!(b.archive.contains(&p));
        }
    }
}
