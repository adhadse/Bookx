//! Pure operations on the flat table of contents (CE-120): move, re-nest, drag and drop, delete, insert.
//!
//! The list is the one of `bookx_core::toc::flatten`: entries in reading order with a `depth`. An entry
//! "owns" the entries after it that are deeper (its subtree); every operation moves a subtree as a whole and
//! keeps the list valid (the first entry at depth 0, a depth never more than one deeper than the entry
//! before). The app writes the result through `toc::write_toc` (nav document and NCX).

use bookx_core::toc::FlatToc;

/// Where a dragged entry lands relative to the entry it is dropped on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DropPos {
    /// As a sibling just before the target.
    Before,
    /// As a sibling just after the target (and its sub-entries).
    After,
    /// As the last child of the target.
    Into,
}

/// One past the last entry of the subtree that starts at `i`.
pub fn subtree_end(toc: &[FlatToc], i: usize) -> usize {
    let Some(first) = toc.get(i) else {
        return toc.len();
    };
    let mut k = i + 1;
    while k < toc.len() && toc[k].depth > first.depth {
        k += 1;
    }
    k
}

/// Makes the depths valid: the first entry at 0, no entry more than one level below the one before.
/// Returns true when something changed.
pub fn normalize(toc: &mut [FlatToc]) -> bool {
    let mut changed = false;
    let mut prev: Option<usize> = None;
    for t in toc.iter_mut() {
        let max = prev.map(|p| p + 1).unwrap_or(0);
        if t.depth > max {
            t.depth = max;
            changed = true;
        }
        prev = Some(t.depth);
    }
    changed
}

/// Problems a reader of the list would notice (for a hint under the list).
pub fn problems(toc: &[FlatToc]) -> Vec<String> {
    let mut out = Vec::new();
    for (i, t) in toc.iter().enumerate() {
        let max = if i == 0 { 0 } else { toc[i - 1].depth + 1 };
        if t.depth > max {
            out.push(format!("entry {} is nested too deep", i + 1));
        }
        if t.title.trim().is_empty() {
            out.push(format!("entry {} has no title", i + 1));
        }
        if t.href.trim().is_empty() {
            out.push(format!("entry {} points nowhere", i + 1));
        }
    }
    out
}

fn shift(entries: &mut [FlatToc], delta: isize) {
    for e in entries {
        e.depth = (e.depth as isize + delta).max(0) as usize;
    }
}

/// Index of the previous sibling of `i` (same depth, same parent).
pub fn prev_sibling(toc: &[FlatToc], i: usize) -> Option<usize> {
    let d = toc.get(i)?.depth;
    (0..i).rev().find_map(|k| match toc[k].depth {
        x if x < d => Some(None),
        x if x == d => Some(Some(k)),
        _ => None,
    })?
}

/// Index of the next sibling of `i`.
pub fn next_sibling(toc: &[FlatToc], i: usize) -> Option<usize> {
    let end = subtree_end(toc, i);
    (end < toc.len() && toc[end].depth == toc[i].depth).then_some(end)
}

/// Index of the parent of `i`.
pub fn parent(toc: &[FlatToc], i: usize) -> Option<usize> {
    let d = toc.get(i)?.depth;
    (0..i).rev().find(|k| toc[*k].depth < d)
}

/// Moves the subtree at `i` before its previous sibling. Returns the new index.
pub fn move_up(toc: &mut Vec<FlatToc>, i: usize) -> Option<usize> {
    let p = prev_sibling(toc, i)?;
    let end = subtree_end(toc, i);
    let moved: Vec<FlatToc> = toc.drain(i..end).collect();
    for (k, e) in moved.into_iter().enumerate() {
        toc.insert(p + k, e);
    }
    Some(p)
}

/// Moves the subtree at `i` after its next sibling. Returns the new index.
pub fn move_down(toc: &mut Vec<FlatToc>, i: usize) -> Option<usize> {
    let n = next_sibling(toc, i)?;
    let n_end = subtree_end(toc, n);
    let len = subtree_end(toc, i) - i;
    let moved: Vec<FlatToc> = toc.drain(i..i + len).collect();
    let at = n_end - len;
    for (k, e) in moved.into_iter().enumerate() {
        toc.insert(at + k, e);
    }
    Some(at)
}

/// Makes the subtree at `i` a child of its previous sibling (appended as its last child).
pub fn demote(toc: &mut [FlatToc], i: usize) -> bool {
    if prev_sibling(toc, i).is_none() {
        return false;
    }
    let end = subtree_end(toc, i);
    shift(&mut toc[i..end], 1);
    true
}

/// Makes the subtree at `i` a sibling of its parent, placed right after the parent's subtree.
/// Returns the new index.
pub fn promote(toc: &mut Vec<FlatToc>, i: usize) -> Option<usize> {
    let p = parent(toc, i)?;
    let end = subtree_end(toc, i);
    let moved: Vec<FlatToc> = toc.drain(i..end).collect();
    let at = subtree_end(toc, p);
    for (k, mut e) in moved.into_iter().enumerate() {
        e.depth -= 1;
        toc.insert(at + k, e);
    }
    Some(at)
}

/// Removes the entry at `i`. With `with_children` its sub-entries go too, otherwise they move up one level.
pub fn delete(toc: &mut Vec<FlatToc>, i: usize, with_children: bool) -> bool {
    if i >= toc.len() {
        return false;
    }
    if with_children {
        let end = subtree_end(toc, i);
        toc.drain(i..end);
    } else {
        let end = subtree_end(toc, i);
        toc.remove(i);
        shift(&mut toc[i..end - 1], -1);
    }
    true
}

/// Inserts `entry` after the subtree of `after` at the depth of `after` (or at the end, depth 0, when `after`
/// is `None`). Returns its index.
pub fn insert_after(toc: &mut Vec<FlatToc>, after: Option<usize>, mut entry: FlatToc) -> usize {
    match after.filter(|i| *i < toc.len()) {
        Some(i) => {
            entry.depth = toc[i].depth;
            let at = subtree_end(toc, i);
            toc.insert(at, entry);
            at
        }
        None => {
            entry.depth = 0;
            toc.push(entry);
            toc.len() - 1
        }
    }
}

/// Drag and drop: moves the subtree at `from` relative to the entry at `target`. Returns the new index of the
/// moved entry, or `None` when the drop makes no sense (onto itself or into its own sub-entries) or changes
/// nothing.
pub fn drop_entry(
    toc: &mut Vec<FlatToc>,
    from: usize,
    target: usize,
    pos: DropPos,
) -> Option<usize> {
    if from >= toc.len() || target >= toc.len() {
        return None;
    }
    let end = subtree_end(toc, from);
    if target >= from && target < end {
        return None;
    }
    let before = toc.clone();
    let mut moved: Vec<FlatToc> = toc.drain(from..end).collect();
    let n = moved.len();
    // the target's index after the removal
    let t = if target >= end { target - n } else { target };
    let (at, depth) = match pos {
        DropPos::Before => (t, toc[t].depth),
        DropPos::After => (subtree_end(toc, t), toc[t].depth),
        DropPos::Into => (subtree_end(toc, t), toc[t].depth + 1),
    };
    let delta = depth as isize - moved[0].depth as isize;
    shift(&mut moved, delta);
    for (k, e) in moved.into_iter().enumerate() {
        toc.insert(at + k, e);
    }
    normalize(toc);
    let same = before.len() == toc.len()
        && before
            .iter()
            .zip(toc.iter())
            .all(|(a, b)| a.depth == b.depth && a.title == b.title && a.href == b.href);
    if same {
        *toc = before;
        return None;
    }
    Some(at)
}

/// Where a pointer at `y` over a row of `height` pixels drops: top quarter before, bottom quarter after, the
/// middle makes a child.
pub fn drop_zone(y: f64, height: f64) -> DropPos {
    if height <= 0.0 {
        return DropPos::After;
    }
    let f = y / height;
    if f < 0.25 {
        DropPos::Before
    } else if f > 0.75 {
        DropPos::After
    } else {
        DropPos::Into
    }
}

/// The anchor an entry added "at the cursor" should point at: the closest element with an `id` that starts at or
/// before `offset` (a byte offset in `src`), and a title taken from it (a heading's text, otherwise the first
/// words after it).
pub fn nearest_anchor(src: &str, offset: usize) -> Option<(String, String)> {
    use crate::markup::{Event, plain_text, scan, tag_attr};
    let mut best: Option<(String, String, usize)> = None;
    for ev in scan(src) {
        if let Event::Open { name, range, .. } = ev {
            if range.start > offset {
                break;
            }
            let Some(id) = tag_attr(&src[range.clone()], "id").filter(|i| !i.is_empty()) else {
                continue;
            };
            let tail = &src[range.end.min(src.len())..];
            let heading =
                name.len() == 2 && name.starts_with('h') && name.as_bytes()[1].is_ascii_digit();
            let text = if heading {
                let close = format!("</{name}");
                let end = tail.find(&close).unwrap_or(tail.len().min(300));
                plain_text(&tail[..end])
            } else {
                plain_text(&tail[..tail.len().min(300)])
            };
            let title: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let title: String = title.chars().take(60).collect();
            best = Some((id, title, range.start));
        }
    }
    best.map(|(i, t, _)| (i, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(spec: &str) -> Vec<FlatToc> {
        // "a b>1 c>1 d e>1": a letter, optionally ">depth"
        spec.split_whitespace()
            .map(|s| {
                let (t, d) = s.split_once('>').unwrap_or((s, "0"));
                FlatToc {
                    depth: d.parse().unwrap(),
                    title: t.to_string(),
                    href: format!("{t}.xhtml"),
                }
            })
            .collect()
    }
    fn show(t: &[FlatToc]) -> String {
        t.iter()
            .map(|e| {
                if e.depth == 0 {
                    e.title.clone()
                } else {
                    format!("{}>{}", e.title, e.depth)
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn subtree_and_siblings() {
        let t = mk("a b>1 c>2 d>1 e f>1");
        assert_eq!(subtree_end(&t, 0), 4);
        assert_eq!(subtree_end(&t, 1), 3);
        assert_eq!(subtree_end(&t, 5), 6);
        assert_eq!(prev_sibling(&t, 3), Some(1));
        assert_eq!(prev_sibling(&t, 1), None);
        assert_eq!(prev_sibling(&t, 4), Some(0));
        assert_eq!(next_sibling(&t, 0), Some(4));
        assert_eq!(parent(&t, 2), Some(1));
        assert_eq!(parent(&t, 0), None);
    }

    #[test]
    fn move_keeps_subtrees_together() {
        let mut t = mk("a b>1 c>1 d e");
        assert_eq!(move_down(&mut t, 0), Some(1));
        assert_eq!(show(&t), "d a b>1 c>1 e");
        assert_eq!(move_up(&mut t, 1), Some(0));
        assert_eq!(show(&t), "a b>1 c>1 d e");
        assert_eq!(move_up(&mut t, 0), None);
        assert_eq!(move_down(&mut t, 4), None);
        assert_eq!(move_down(&mut t, 1), Some(2));
        assert_eq!(show(&t), "a c>1 b>1 d e");
        assert_eq!(
            move_down(&mut t, 2),
            None,
            "the last child does not leave its parent"
        );
    }

    #[test]
    fn demote_and_promote() {
        let mut t = mk("a b c>1");
        assert!(demote(&mut t, 1));
        assert_eq!(show(&t), "a b>1 c>2");
        assert!(!demote(&mut t, 0));
        assert!(!demote(&mut t, 2), "the only child has no sibling to join");
        let mut t = mk("a b>1 c>1 d>1 e");
        assert_eq!(promote(&mut t, 1), Some(3));
        assert_eq!(show(&t), "a c>1 d>1 b e");
        assert_eq!(promote(&mut t, 3), None, "a top entry cannot be promoted");
        let mut t = mk("a b>1 c>2 d>1");
        assert_eq!(promote(&mut t, 1), Some(2));
        assert_eq!(show(&t), "a d>1 b c>1");
    }

    #[test]
    fn delete_variants() {
        let mut t = mk("a b>1 c>2 d>1 e");
        assert!(delete(&mut t, 1, false));
        assert_eq!(show(&t), "a c>1 d>1 e");
        let mut t = mk("a b>1 c>2 d>1 e");
        assert!(delete(&mut t, 1, true));
        assert_eq!(show(&t), "a d>1 e");
        assert!(!delete(&mut t, 9, true));
        let mut t = mk("a b>1");
        assert!(delete(&mut t, 0, false));
        assert_eq!(show(&t), "b", "children of a removed top entry move up");
    }

    #[test]
    fn insert_after_uses_the_subtree_end() {
        let mut t = mk("a b>1 c");
        let i = insert_after(
            &mut t,
            Some(0),
            FlatToc {
                depth: 5,
                title: "n".into(),
                href: "n.xhtml".into(),
            },
        );
        assert_eq!(i, 2);
        assert_eq!(show(&t), "a b>1 n c");
        let i = insert_after(
            &mut t,
            None,
            FlatToc {
                depth: 3,
                title: "z".into(),
                href: String::new(),
            },
        );
        assert_eq!((i, show(&t).as_str()), (4, "a b>1 n c z"));
    }

    #[test]
    fn drop_rules() {
        let mut t = mk("a b>1 c");
        assert_eq!(
            drop_entry(&mut t, 0, 1, DropPos::After),
            None,
            "into itself"
        );
        assert_eq!(drop_entry(&mut t, 0, 0, DropPos::Before), None);
        assert_eq!(drop_entry(&mut t, 0, 9, DropPos::Before), None);
        assert_eq!(drop_entry(&mut t, 2, 1, DropPos::After), Some(2));
        assert_eq!(show(&t), "a b>1 c>1");
        assert_eq!(drop_zone(1.0, 40.0), DropPos::Before);
        assert_eq!(drop_zone(20.0, 40.0), DropPos::Into);
        assert_eq!(drop_zone(39.0, 40.0), DropPos::After);
        let mut bad = mk("a b>1");
        bad[0].depth = 2;
        assert!(normalize(&mut bad));
        assert_eq!(show(&bad), "a b>1");
        assert!(!normalize(&mut bad));
        assert!(problems(&mk("a b>1")).is_empty());
        let mut x = mk("a");
        x[0].title.clear();
        x[0].href.clear();
        assert_eq!(problems(&x).len(), 2);
    }

    #[test]
    fn drop_before_after_into() {
        let mut t = mk("a b>1 c d");
        assert_eq!(drop_entry(&mut t, 3, 0, DropPos::Before), Some(0));
        assert_eq!(show(&t), "d a b>1 c");
        let mut t = mk("a b>1 c d");
        assert_eq!(drop_entry(&mut t, 0, 3, DropPos::After), Some(2));
        assert_eq!(show(&t), "c d a b>1");
        let mut t = mk("a b>1 c d");
        assert_eq!(drop_entry(&mut t, 0, 3, DropPos::Into), Some(2));
        assert_eq!(show(&t), "c d a>1 b>2");
        let mut t = mk("a b>1 c d");
        assert_eq!(drop_entry(&mut t, 2, 1, DropPos::Before), Some(1));
        assert_eq!(
            show(&t),
            "a c>1 b>1 d",
            "dropping on a child re-nests the entry"
        );
        let mut t = mk("a b>1 c d");
        assert_eq!(drop_entry(&mut t, 3, 0, DropPos::Into), Some(2));
        assert_eq!(show(&t), "a b>1 d>1 c");
    }

    #[test]
    fn nearest_anchor_finds_the_element_before_the_cursor() {
        let src = "<body><h2 id=\"a\">First <i>part</i></h2><p>text</p><div id=\"b\"><p>Some words here</p></div><p>end</p></body>";
        assert_eq!(nearest_anchor(src, 0), None);
        let at = src.find("<p>text").unwrap();
        assert_eq!(
            nearest_anchor(src, at),
            Some(("a".into(), "First part".into()))
        );
        let at = src.find("<p>end").unwrap();
        let (id, title) = nearest_anchor(src, at).unwrap();
        assert_eq!(id, "b");
        assert!(title.starts_with("Some words here"), "{title}");
        assert_eq!(nearest_anchor("<p>x</p>", 5), None);
    }
}
