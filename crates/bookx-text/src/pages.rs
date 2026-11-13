//! Page numbering (R24): synthetic pages from character weights, real pages from an EPUB `page-list`,
//! hidden flows, position <-> page mapping and the layout-based page estimate used by the reader.

use std::collections::HashMap;

/// Default synthetic page size in characters.
pub const DEFAULT_CHARS_PER_PAGE: f64 = 2000.0;

/// A page-list entry inside a spine item.
#[derive(Debug, Clone, PartialEq)]
pub struct PageTarget {
    /// The printed page label (`"iv"`, `"23"`).
    pub label: String,
    /// Character offset inside the spine item where the page starts.
    pub offset: f64,
}

impl PageTarget {
    pub fn new(label: impl Into<String>, offset: f64) -> Self {
        PageTarget {
            label: label.into(),
            offset,
        }
    }
}

/// One spine entry.
#[derive(Debug, Clone, PartialEq)]
pub struct SpineItem {
    /// Amount of text (characters). Negative or NaN values count as 0.
    pub weight: f64,
    /// Hidden flows (front matter, ads, notes) do not count towards pages or progress.
    pub hidden: bool,
    /// EPUB page-list targets inside this item.
    pub targets: Vec<PageTarget>,
}

impl SpineItem {
    pub fn new(weight: f64) -> Self {
        SpineItem {
            weight: sane(weight),
            hidden: false,
            targets: Vec::new(),
        }
    }

    pub fn hidden(weight: f64) -> Self {
        SpineItem {
            hidden: true,
            ..SpineItem::new(weight)
        }
    }

    pub fn with_targets(mut self, targets: Vec<PageTarget>) -> Self {
        self.targets = targets;
        self
    }
}

fn sane(w: f64) -> f64 {
    if w.is_finite() && w > 0.0 {
        w
    } else {
        0.0
    }
}

/// Which numbering to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PageSource {
    /// Use the page-list when the visible part of the book has one, otherwise synthetic pages.
    #[default]
    Auto,
    Synthetic,
    /// Only the page-list; no pages when it is absent.
    PageList,
}

/// The numbering actually in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageMode {
    Synthetic,
    PageList,
}

/// A resolved page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    /// 1-based position in the numbering.
    pub number: usize,
    /// Printed label (equals `number` for synthetic pages).
    pub label: String,
}

#[derive(Debug, Clone)]
struct Target {
    spine: usize,
    offset: f64,
    label: String,
}

/// Page map of a book.
#[derive(Debug, Clone)]
pub struct PageMap {
    mode: PageMode,
    chars_per_page: f64,
    weights: Vec<f64>,
    hidden: Vec<bool>,
    before: Vec<f64>,
    visible_total: f64,
    targets: Vec<Target>,
}

impl PageMap {
    /// Build a page map. `chars_per_page` <= 0 uses [`DEFAULT_CHARS_PER_PAGE`].
    pub fn new(items: &[SpineItem], chars_per_page: f64, source: PageSource) -> PageMap {
        let cpp = if chars_per_page.is_finite() && chars_per_page > 0.0 {
            chars_per_page
        } else {
            DEFAULT_CHARS_PER_PAGE
        };
        let weights: Vec<f64> = items.iter().map(|i| sane(i.weight)).collect();
        let hidden: Vec<bool> = items.iter().map(|i| i.hidden).collect();
        let mut before = Vec::with_capacity(items.len());
        let mut acc = 0.0;
        for (w, h) in weights.iter().zip(&hidden) {
            before.push(acc);
            if !h {
                acc += w;
            }
        }
        let mut targets = Vec::new();
        for (i, it) in items.iter().enumerate() {
            if it.hidden {
                continue;
            }
            let mut ts: Vec<&PageTarget> = it.targets.iter().collect();
            ts.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            for t in ts {
                targets.push(Target {
                    spine: i,
                    offset: sane(t.offset).min(weights[i]),
                    label: t.label.clone(),
                });
            }
        }
        let mode = match source {
            PageSource::Synthetic => PageMode::Synthetic,
            PageSource::PageList => PageMode::PageList,
            PageSource::Auto => {
                if targets.is_empty() {
                    PageMode::Synthetic
                } else {
                    PageMode::PageList
                }
            }
        };
        PageMap {
            mode,
            chars_per_page: cpp,
            weights,
            hidden,
            before,
            visible_total: acc,
            targets,
        }
    }

    pub fn mode(&self) -> PageMode {
        self.mode
    }

    pub fn chars_per_page(&self) -> f64 {
        self.chars_per_page
    }

    fn has_visible(&self) -> bool {
        self.hidden.iter().any(|h| !h)
    }

    /// Number of pages (0 when there is nothing visible, or in page-list mode without a page-list).
    pub fn total_pages(&self) -> usize {
        match self.mode {
            PageMode::PageList => self.targets.len(),
            PageMode::Synthetic => {
                if !self.has_visible() {
                    0
                } else {
                    ((self.visible_total / self.chars_per_page - 1e-9).ceil() as usize).max(1)
                }
            }
        }
    }

    pub fn is_hidden(&self, spine: usize) -> bool {
        self.hidden.get(spine).copied().unwrap_or(true)
    }

    /// Fraction (0..=1) of the visible text before `offset` in spine item `spine`. `None` for hidden or unknown items.
    pub fn progress(&self, spine: usize, offset: f64) -> Option<f64> {
        if self.is_hidden(spine) {
            return None;
        }
        if self.visible_total <= 0.0 {
            return Some(0.0);
        }
        let off = sane(offset).min(self.weights[spine]);
        Some(((self.before[spine] + off) / self.visible_total).clamp(0.0, 1.0))
    }

    /// The page containing `offset` of spine item `spine`. `None` for hidden or unknown items, or when no pages exist.
    pub fn page_at(&self, spine: usize, offset: f64) -> Option<PageRef> {
        if self.is_hidden(spine) || self.total_pages() == 0 {
            return None;
        }
        let off = sane(offset).min(self.weights[spine]);
        match self.mode {
            PageMode::Synthetic => {
                let g = self.before[spine] + off;
                let n =
                    ((g / self.chars_per_page + 1e-9).floor() as usize + 1).min(self.total_pages());
                Some(PageRef {
                    number: n,
                    label: n.to_string(),
                })
            }
            PageMode::PageList => {
                let idx = self
                    .targets
                    .partition_point(|t| t.spine < spine || (t.spine == spine && t.offset <= off));
                let i = idx.saturating_sub(1);
                Some(PageRef {
                    number: i + 1,
                    label: self.targets[i].label.clone(),
                })
            }
        }
    }

    /// Spine item and character offset where page `number` (1-based) starts.
    pub fn position_of(&self, number: usize) -> Option<(usize, f64)> {
        if number == 0 || number > self.total_pages() {
            return None;
        }
        match self.mode {
            PageMode::PageList => {
                let t = &self.targets[number - 1];
                Some((t.spine, t.offset))
            }
            PageMode::Synthetic => {
                let g = (number - 1) as f64 * self.chars_per_page;
                let mut last_visible = None;
                for i in 0..self.weights.len() {
                    if self.hidden[i] {
                        continue;
                    }
                    last_visible = Some(i);
                    if self.before[i] + self.weights[i] > g + 1e-9 {
                        return Some((i, (g - self.before[i]).max(0.0)));
                    }
                }
                last_visible.map(|i| (i, self.weights[i]))
            }
        }
    }

    /// First page whose label equals `label` (page-list mode; synthetic labels are the numbers).
    pub fn position_of_label(&self, label: &str) -> Option<(usize, f64)> {
        match self.mode {
            PageMode::PageList => self
                .targets
                .iter()
                .position(|t| t.label == label)
                .and_then(|i| self.position_of(i + 1)),
            PageMode::Synthetic => label
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|n| self.position_of(n)),
        }
    }

    /// Page label for a position given as a fraction of the spine item.
    pub fn page_at_fraction(&self, spine: usize, frac: f64) -> Option<PageRef> {
        let w = *self.weights.get(spine)?;
        let f = if frac.is_finite() {
            frac.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.page_at(spine, f * w)
    }
}

/// Book-wide page estimate from layout results of the chapters that were laid out so far.
///
/// `known` maps spine index to the real page count of that chapter, `weights` are the text weights of all
/// chapters, `idx` is the current chapter and `page`/`pages` the (1-based) page inside it. Chapters without a
/// layout are estimated from the density (pages per weight) of the known ones, falling back to one page per
/// 2000 characters. Returns `(current page of the book, total pages)`.
pub fn estimate_pages(
    known: &HashMap<usize, usize>,
    weights: &[f64],
    idx: usize,
    page: usize,
    pages: usize,
) -> (usize, usize) {
    estimate_pages_visible(known, weights, &[], idx, page, pages).unwrap_or((page, pages))
}

/// Like [`estimate_pages`], but chapters flagged in `hidden` neither count towards the page nor the total.
/// Returns `None` when the current chapter is hidden.
pub fn estimate_pages_visible(
    known: &HashMap<usize, usize>,
    weights: &[f64],
    hidden: &[bool],
    idx: usize,
    page: usize,
    pages: usize,
) -> Option<(usize, usize)> {
    let is_hidden = |i: usize| hidden.get(i).copied().unwrap_or(false);
    if is_hidden(idx) {
        return None;
    }
    let (mut kp, mut kw) = (0.0, 0.0);
    for (i, p) in known.iter() {
        if is_hidden(*i) {
            continue;
        }
        kp += *p as f64;
        kw += weights.get(*i).copied().unwrap_or(0.0);
    }
    let density = if kw > 0.0 {
        kp / kw
    } else {
        1.0 / DEFAULT_CHARS_PER_PAGE
    };
    let pages_of = |i: usize| {
        if is_hidden(i) {
            return 0.0;
        }
        known
            .get(&i)
            .map(|p| *p as f64)
            .unwrap_or_else(|| density * weights.get(i).copied().unwrap_or(0.0))
    };
    let before = (0..idx).map(pages_of).sum::<f64>().round() as usize;
    let total = (0..weights.len()).map(pages_of).sum::<f64>().round() as usize;
    Some((before + page, total.max(before + pages)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(items: &[SpineItem]) -> PageMap {
        PageMap::new(items, 1000.0, PageSource::Auto)
    }

    #[test]
    fn synthetic_totals_and_page_at() {
        let m = synth(&[SpineItem::new(2500.0), SpineItem::new(1500.0)]);
        assert_eq!(m.mode(), PageMode::Synthetic);
        assert_eq!(m.total_pages(), 4);
        assert_eq!(m.page_at(0, 0.0).unwrap().number, 1);
        assert_eq!(m.page_at(0, 999.0).unwrap().number, 1);
        assert_eq!(m.page_at(0, 1000.0).unwrap().number, 2);
        assert_eq!(m.page_at(0, 2499.0).unwrap().number, 3);
        assert_eq!(m.page_at(1, 0.0).unwrap().number, 3);
        assert_eq!(m.page_at(1, 500.0).unwrap().number, 4);
        assert_eq!(
            m.page_at(1, 1500.0).unwrap().number,
            4,
            "end of book stays on the last page"
        );
        assert_eq!(m.page_at(1, 1500.0).unwrap().label, "4");
    }

    #[test]
    fn exact_multiple_does_not_add_extra_page() {
        let m = synth(&[SpineItem::new(2000.0)]);
        assert_eq!(m.total_pages(), 2);
        assert_eq!(m.page_at(0, 2000.0).unwrap().number, 2);
    }

    #[test]
    fn synthetic_position_of_roundtrip() {
        let m = synth(&[
            SpineItem::new(2500.0),
            SpineItem::new(0.0),
            SpineItem::new(1500.0),
            SpineItem::new(10.0),
        ]);
        assert_eq!(m.total_pages(), 5);
        for p in 1..=m.total_pages() {
            let (i, off) = m.position_of(p).unwrap();
            assert_eq!(
                m.page_at(i, off).unwrap().number,
                p,
                "page {p} -> ({i}, {off})"
            );
        }
        assert_eq!(m.position_of(1), Some((0, 0.0)));
        assert_eq!(m.position_of(3), Some((0, 2000.0)));
        assert_eq!(m.position_of(4), Some((2, 500.0)));
        assert_eq!(m.position_of(0), None);
        assert_eq!(m.position_of(6), None);
    }

    #[test]
    fn hidden_flows_are_excluded() {
        let m = synth(&[
            SpineItem::hidden(3000.0),
            SpineItem::new(1000.0),
            SpineItem::hidden(500.0),
            SpineItem::new(1000.0),
        ]);
        assert_eq!(m.total_pages(), 2);
        assert!(m.page_at(0, 10.0).is_none());
        assert!(m.page_at(2, 10.0).is_none());
        assert_eq!(m.page_at(1, 0.0).unwrap().number, 1);
        assert_eq!(m.page_at(3, 0.0).unwrap().number, 2);
        assert_eq!(m.position_of(2), Some((3, 0.0)));
        assert!(m.is_hidden(0) && !m.is_hidden(1) && m.is_hidden(99));
        assert_eq!(m.progress(0, 5.0), None);
        assert_eq!(m.progress(1, 0.0), Some(0.0));
        assert_eq!(m.progress(3, 1000.0), Some(1.0));
        assert_eq!(m.progress(3, 500.0), Some(0.75));
    }

    #[test]
    fn everything_hidden_or_empty() {
        for items in [vec![], vec![SpineItem::hidden(100.0)]] {
            let m = synth(&items);
            assert_eq!(m.total_pages(), 0);
            assert!(m.page_at(0, 0.0).is_none());
            assert!(m.position_of(1).is_none());
            assert_eq!(m.progress(0, 0.0), None);
        }
        let m = synth(&[SpineItem::new(0.0)]);
        assert_eq!(m.total_pages(), 1);
        assert_eq!(m.page_at(0, 0.0).unwrap().number, 1);
        assert_eq!(m.progress(0, 0.0), Some(0.0));
        assert_eq!(m.position_of(1), Some((0, 0.0)));
    }

    #[test]
    fn invalid_weights_and_cpp_are_sanitised() {
        let m = PageMap::new(
            &[
                SpineItem::new(f64::NAN),
                SpineItem::new(-5.0),
                SpineItem::new(f64::INFINITY),
                SpineItem::new(100.0),
            ],
            -1.0,
            PageSource::Auto,
        );
        assert_eq!(m.chars_per_page(), DEFAULT_CHARS_PER_PAGE);
        assert_eq!(m.total_pages(), 1);
        let m = PageMap::new(&[SpineItem::new(10.0)], f64::NAN, PageSource::Synthetic);
        assert_eq!(m.chars_per_page(), DEFAULT_CHARS_PER_PAGE);
        // offsets are clamped
        let m = synth(&[SpineItem::new(1500.0)]);
        assert_eq!(m.page_at(0, -50.0).unwrap().number, 1);
        assert_eq!(m.page_at(0, 1e12).unwrap().number, 2);
        assert_eq!(m.page_at(0, f64::NAN).unwrap().number, 1);
    }

    fn with_list() -> Vec<SpineItem> {
        vec![
            SpineItem::new(500.0),
            SpineItem::new(3000.0).with_targets(vec![
                PageTarget::new("3", 1000.0),
                PageTarget::new("1", 0.0),
                PageTarget::new("2", 500.0),
            ]),
            SpineItem::hidden(100.0).with_targets(vec![PageTarget::new("hidden", 0.0)]),
            SpineItem::new(1000.0).with_targets(vec![PageTarget::new("4", 100.0)]),
        ]
    }

    #[test]
    fn page_list_mode_is_chosen_automatically_and_sorted() {
        let m = PageMap::new(&with_list(), 1000.0, PageSource::Auto);
        assert_eq!(m.mode(), PageMode::PageList);
        assert_eq!(m.total_pages(), 4, "hidden targets are ignored");
        assert_eq!(m.position_of(1), Some((1, 0.0)));
        assert_eq!(m.position_of(2), Some((1, 500.0)));
        assert_eq!(m.position_of(3), Some((1, 1000.0)));
        assert_eq!(m.position_of(4), Some((3, 100.0)));
    }

    #[test]
    fn page_list_page_at_and_labels() {
        let m = PageMap::new(&with_list(), 1000.0, PageSource::Auto);
        let at = |i, o| m.page_at(i, o).map(|p| (p.number, p.label));
        assert_eq!(at(1, 0.0), Some((1, "1".into())));
        assert_eq!(at(1, 499.0), Some((1, "1".into())));
        assert_eq!(at(1, 500.0), Some((2, "2".into())));
        assert_eq!(at(1, 2999.0), Some((3, "3".into())));
        assert_eq!(
            at(3, 50.0),
            Some((3, "3".into())),
            "before the first target of an item keeps the previous page"
        );
        assert_eq!(at(3, 100.0), Some((4, "4".into())));
        assert_eq!(
            at(0, 0.0),
            Some((1, "1".into())),
            "front matter before page 1 shows the first page"
        );
        assert_eq!(at(2, 0.0), None);
        assert_eq!(at(9, 0.0), None);
    }

    #[test]
    fn page_list_labels_lookup() {
        let m = PageMap::new(&with_list(), 1000.0, PageSource::Auto);
        assert_eq!(m.position_of_label("2"), Some((1, 500.0)));
        assert_eq!(m.position_of_label("4"), Some((3, 100.0)));
        assert_eq!(m.position_of_label("hidden"), None);
        assert_eq!(m.position_of_label("zz"), None);
        let s = synth(&[SpineItem::new(5000.0)]);
        assert_eq!(s.position_of_label(" 3 "), Some((0, 2000.0)));
        assert_eq!(s.position_of_label("iii"), None);
        assert_eq!(s.position_of_label("9"), None);
    }

    #[test]
    fn page_source_overrides() {
        let items = with_list();
        let syn = PageMap::new(&items, 1000.0, PageSource::Synthetic);
        assert_eq!(syn.mode(), PageMode::Synthetic);
        assert_eq!(syn.total_pages(), 5, "hidden item excluded: 4500 chars");
        let plain = vec![SpineItem::new(2000.0)];
        let forced = PageMap::new(&plain, 1000.0, PageSource::PageList);
        assert_eq!(forced.mode(), PageMode::PageList);
        assert_eq!(forced.total_pages(), 0);
        assert!(forced.page_at(0, 0.0).is_none());
        assert_eq!(
            PageMap::new(&plain, 1000.0, PageSource::Auto).mode(),
            PageMode::Synthetic
        );
        assert_eq!(PageSource::default(), PageSource::Auto);
    }

    #[test]
    fn page_list_target_offsets_clamped_to_item() {
        let items = vec![SpineItem::new(100.0).with_targets(vec![
            PageTarget::new("a", -5.0),
            PageTarget::new("b", 5000.0),
        ])];
        let m = PageMap::new(&items, 1000.0, PageSource::Auto);
        assert_eq!(m.position_of(1), Some((0, 0.0)));
        assert_eq!(m.position_of(2), Some((0, 100.0)));
    }

    #[test]
    fn progress_fraction() {
        let m = synth(&[SpineItem::new(1000.0), SpineItem::new(3000.0)]);
        assert_eq!(m.progress(0, 0.0), Some(0.0));
        assert_eq!(m.progress(0, 1000.0), Some(0.25));
        assert_eq!(m.progress(1, 1500.0), Some(0.625));
        assert_eq!(m.progress(1, 1e9), Some(1.0));
        assert_eq!(m.progress(7, 0.0), None);
    }

    #[test]
    fn page_at_fraction_uses_weight() {
        let m = synth(&[SpineItem::new(1000.0), SpineItem::new(3000.0)]);
        assert_eq!(m.page_at_fraction(1, 0.5).unwrap().number, 3);
        assert_eq!(m.page_at_fraction(1, 1.0).unwrap().number, 4);
        assert_eq!(m.page_at_fraction(1, 7.0).unwrap().number, 4);
        assert_eq!(m.page_at_fraction(1, f64::NAN).unwrap().number, 2);
        assert!(m.page_at_fraction(5, 0.5).is_none());
    }

    #[test]
    fn estimate_matches_reader_semantics() {
        // same fixture and expectations as `estimate_book_pages` in src/ui/reader.rs
        let mut known = HashMap::new();
        known.insert(0usize, 10usize);
        let weights = [1000.0, 1000.0, 2000.0];
        assert_eq!(estimate_pages(&known, &weights, 1, 3, 10), (13, 40));
        // a second measured chapter refines the density
        known.insert(2usize, 30usize);
        assert_eq!(estimate_pages(&known, &weights, 1, 3, 10), (13, 53));
        assert_eq!(estimate_pages(&HashMap::new(), &[1.0], 0, 5, 7), (5, 7));
    }

    #[test]
    fn estimate_without_known_uses_default_density() {
        let weights = [4000.0, 4000.0, 2000.0];
        let (page, total) = estimate_pages(&HashMap::new(), &weights, 1, 1, 2);
        assert_eq!((page, total), (3, 5));
        // current chapter's own count wins if bigger than the estimate
        let (_, total) = estimate_pages(&HashMap::new(), &weights, 1, 1, 50);
        assert_eq!(total, 52);
        // unknown weights count as zero
        assert_eq!(estimate_pages(&HashMap::new(), &[], 0, 1, 1), (1, 1));
    }

    #[test]
    fn estimate_visible_skips_hidden() {
        let mut known = HashMap::new();
        known.insert(1usize, 10usize);
        let weights = [2000.0, 1000.0, 3000.0];
        let hidden = [true, false, false];
        // density 10/1000; chapter 2 -> 30 pages; chapter 0 hidden does not count
        assert_eq!(
            estimate_pages_visible(&known, &weights, &hidden, 2, 4, 30),
            Some((14, 40))
        );
        assert_eq!(
            estimate_pages_visible(&known, &weights, &hidden, 0, 1, 5),
            None
        );
        // hidden known chapters do not skew the density
        known.insert(0, 1000);
        assert_eq!(
            estimate_pages_visible(&known, &weights, &hidden, 2, 1, 1),
            Some((11, 40))
        );
        // no hidden flags behaves like estimate_pages
        let none: [bool; 0] = [];
        assert_eq!(
            estimate_pages_visible(&known, &weights, &none, 1, 2, 10),
            Some(estimate_pages(&known, &weights, 1, 2, 10))
        );
    }

    #[test]
    fn property_roundtrip_random_books() {
        let mut seed = 31337u64;
        let mut rnd = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for _ in 0..300 {
            let n = 1 + (rnd() % 12) as usize;
            let items: Vec<SpineItem> = (0..n)
                .map(|_| {
                    let w = (rnd() % 5000) as f64;
                    if rnd() % 4 == 0 {
                        SpineItem::hidden(w)
                    } else {
                        SpineItem::new(w)
                    }
                })
                .collect();
            let cpp = 200.0 + (rnd() % 2000) as f64;
            let m = PageMap::new(&items, cpp, PageSource::Synthetic);
            let total = m.total_pages();
            let visible: f64 = items.iter().filter(|i| !i.hidden).map(|i| i.weight).sum();
            if items.iter().all(|i| i.hidden) {
                assert_eq!(total, 0);
                continue;
            }
            assert_eq!(total, ((visible / cpp - 1e-9).ceil() as usize).max(1));
            let mut prev = 0;
            for p in 1..=total {
                let (i, off) = m.position_of(p).unwrap();
                assert!(!m.is_hidden(i));
                assert_eq!(m.page_at(i, off).unwrap().number, p);
                assert!(p > prev);
                prev = p;
            }
            // monotonic along the book
            let mut last = 0;
            for (i, it) in items.iter().enumerate() {
                if it.hidden {
                    continue;
                }
                for k in 0..=4 {
                    let n = m.page_at(i, it.weight * k as f64 / 4.0).unwrap().number;
                    assert!(n >= last);
                    last = n;
                }
            }
        }
    }
}
