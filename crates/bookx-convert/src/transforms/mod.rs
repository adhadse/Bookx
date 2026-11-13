//! Transforms: one deterministic, idempotent stage of the conversion pipeline each.

use std::thread;

use anyhow::Result;
use bookx_core::book::OpenBook;
use bookx_core::contracts::{Registry, Transform};

pub mod css;
pub mod device;
pub mod heuristics;
pub mod metadata;
pub mod punctuation;
pub mod split;
pub mod toc;
pub mod unwrap;

pub use css::RemoveUnusedCss;
pub use device::{DeviceProfile, ProfileKind};
pub use heuristics::{RemoveBlankParagraphs, RemoveFakeMargins, ReplaceSceneBreaks};
pub use metadata::{MetadataOverride, SetMetadataOverride};
pub use punctuation::{SmartenPunctuation, UnsmartenPunctuation};
pub use split::SplitLargeHtml;
pub use toc::{AutoToc, AutoTocMode};
pub use unwrap::UnwrapLines;

/// Number of worker threads to use when `jobs == 0`.
pub fn default_jobs() -> usize {
    thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1)
}

/// Runs `f(path, text)` on every spine document and stores the results that differ.
/// `f` returns `None` (or the unchanged text) when nothing changed. Work is spread over `jobs`
/// threads (`0` = all cores); the result never depends on the thread count.
pub fn map_spine_files<F>(book: &mut OpenBook, jobs: usize, f: F) -> Result<usize>
where
    F: Fn(&str, &str) -> Option<String> + Sync,
{
    let jobs = if jobs == 0 { default_jobs() } else { jobs };
    let inputs: Vec<(String, String)> = book
        .spine_paths()
        .into_iter()
        .filter_map(|p| book.archive.get_string(&p).map(|t| (p, t)))
        .collect();
    let mut results: Vec<Option<String>> = vec![None; inputs.len()];
    if jobs <= 1 || inputs.len() <= 1 {
        for (i, (p, t)) in inputs.iter().enumerate() {
            results[i] = f(p, t);
        }
    } else {
        let chunk = inputs.len().div_ceil(jobs);
        thread::scope(|s| {
            for (ins, outs) in inputs.chunks(chunk).zip(results.chunks_mut(chunk)) {
                let f = &f;
                s.spawn(move || {
                    for ((p, t), o) in ins.iter().zip(outs.iter_mut()) {
                        *o = f(p, t);
                    }
                });
            }
        });
    }
    let mut changed = 0;
    for ((path, old), new) in inputs.iter().zip(results) {
        if let Some(new) = new {
            if new != *old {
                book.archive.set_string(path, &new);
                changed += 1;
            }
        }
    }
    Ok(changed)
}

/// All transforms with default settings, in a sensible order. Mostly useful for listing ids.
pub fn default_transforms() -> Registry<dyn Transform> {
    let mut r: Registry<dyn Transform> = Registry::new();
    r.register(Box::new(UnwrapLines::default()));
    r.register(Box::new(RemoveBlankParagraphs::default()));
    r.register(Box::new(RemoveFakeMargins));
    r.register(Box::new(ReplaceSceneBreaks::default()));
    r.register(Box::new(SmartenPunctuation::default()));
    r.register(Box::new(UnsmartenPunctuation::default()));
    r.register(Box::new(RemoveUnusedCss));
    r.register(Box::new(SplitLargeHtml::default()));
    r.register(Box::new(AutoToc::default()));
    r.register(Box::new(SetMetadataOverride::default()));
    r.register(Box::new(DeviceProfile::generic_eink()));
    r
}

#[cfg(test)]
pub(crate) mod testutil {
    use bookx_core::archive::Archive;
    use bookx_core::book::OpenBook;
    use bookx_core::convert::{create_epub, Chapter};
    use bookx_core::opf::Metadata;

    pub fn book_with(chapters: &[(&str, &str)]) -> OpenBook {
        let md = Metadata {
            title: "Test Book".into(),
            authors: vec!["Ann Author".into()],
            language: "en".into(),
            ..Default::default()
        };
        let chs: Vec<Chapter> = chapters
            .iter()
            .map(|(t, b)| Chapter {
                title: t.to_string(),
                body: b.to_string(),
            })
            .collect();
        let arc = create_epub(&md, &chs, None);
        // round-trip through bytes like a real file
        let bytes = arc.to_bytes().expect("zip");
        OpenBook::from_archive(Archive::from_bytes(bytes).expect("zip")).expect("epub")
    }

    pub fn chapter(book: &OpenBook, n: usize) -> String {
        let p = book.spine_paths()[n].clone();
        book.archive.get_string(&p).expect("chapter")
    }

    /// File names with a hash of their content (short Debug output when a comparison fails).
    pub fn snapshot(book: &OpenBook) -> Vec<(String, u64)> {
        use std::hash::{Hash, Hasher};
        let mut v: Vec<(String, u64)> = book
            .archive
            .names()
            .iter()
            .map(|n| {
                let mut h = std::collections::hash_map::DefaultHasher::new();
                book.archive.get(n).unwrap_or_default().hash(&mut h);
                (n.clone(), h.finish())
            })
            .collect();
        v.sort();
        v
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;

    #[test]
    fn map_spine_files_counts_only_changes() {
        let mut b = book_with(&[("A", "<p>a</p>"), ("B", "<p>b</p>")]);
        let n = map_spine_files(&mut b, 1, |_, t| {
            if t.contains("<p>a</p>") {
                Some(t.replace("<p>a</p>", "<p>A</p>"))
            } else {
                Some(t.to_string())
            }
        })
        .unwrap();
        assert_eq!(n, 1);
        assert!(chapter(&b, 0).contains("<p>A</p>"));
        assert!(chapter(&b, 1).contains("<p>b</p>"));
    }

    #[test]
    fn map_spine_files_is_independent_of_job_count() {
        let chapters: Vec<(String, String)> = (0..9)
            .map(|i| (format!("C{i}"), format!("<p>text {i}</p>")))
            .collect();
        let refs: Vec<(&str, &str)> = chapters
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        let mut seq = book_with(&refs);
        let mut par = book_with(&refs);
        let f = |p: &str, t: &str| Some(t.replace("text", &format!("TEXT@{}", p.len())));
        let a = map_spine_files(&mut seq, 1, f).unwrap();
        let b = map_spine_files(&mut par, 4, f).unwrap();
        assert_eq!(a, 9);
        assert_eq!(b, 9);
        for i in 0..9 {
            assert_eq!(chapter(&seq, i), chapter(&par, i));
        }
        let mut all = book_with(&refs);
        assert_eq!(map_spine_files(&mut all, 0, f).unwrap(), 9);
    }

    #[test]
    fn registry_lists_every_transform() {
        let r = default_transforms();
        let ids: Vec<&str> = r.iter().map(|t| t.id()).collect();
        for want in [
            "unwrap_lines",
            "remove_blank_paragraphs",
            "remove_fake_margins",
            "replace_scene_breaks",
            "smarten_punctuation",
            "unsmarten_punctuation",
            "remove_unused_css",
            "split_large_html",
            "auto_toc",
            "set_metadata_override",
            "device_profile",
        ] {
            assert!(ids.contains(&want), "missing {want}");
        }
        let mut sorted = ids.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), ids.len(), "ids must be unique");
    }
}
