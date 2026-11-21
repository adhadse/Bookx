//! Collapsing the table of contents in the reader's sidebar (roadmap R28): which entries are
//! shown when some parents are folded. Entries come in reading order with a depth; an entry is a
//! parent when the next one is deeper.

use std::collections::BTreeSet;

/// Whether entry `i` has entries under it.
pub fn is_parent(depths: &[usize], i: usize) -> bool {
    matches!((depths.get(i), depths.get(i + 1)), (Some(a), Some(b)) if b > a)
}

/// Index of the nearest entry before `i` that is shallower (its parent), if any.
pub fn parent_of(depths: &[usize], i: usize) -> Option<usize> {
    let d = *depths.get(i)?;
    (0..i).rev().find(|&j| depths[j] < d)
}

/// The entries that are shown: those with no folded ancestor.
pub fn visible(depths: &[usize], folded: &BTreeSet<usize>) -> Vec<usize> {
    let mut out = Vec::with_capacity(depths.len());
    // depth below which entries are hidden, while inside a folded parent
    let mut hide_deeper_than: Option<usize> = None;
    for (i, &d) in depths.iter().enumerate() {
        if let Some(limit) = hide_deeper_than {
            if d > limit {
                continue;
            }
            hide_deeper_than = None;
        }
        out.push(i);
        if folded.contains(&i) && is_parent(depths, i) {
            hide_deeper_than = Some(d);
        }
    }
    out
}

/// Where entry `i` shows in `visible`: itself, or the nearest shown ancestor when it is hidden.
pub fn position_of(depths: &[usize], visible: &[usize], i: usize) -> Option<usize> {
    let mut at = Some(i);
    while let Some(j) = at {
        if let Ok(p) = visible.binary_search(&j) {
            return Some(p);
        }
        at = parent_of(depths, j);
    }
    None
}

/// Every parent folded.
pub fn fold_all(depths: &[usize]) -> BTreeSet<usize> {
    (0..depths.len())
        .filter(|&i| is_parent(depths, i))
        .collect()
}

/// Unfolds the ancestors of `i` so that it shows (the chapter being read is never hidden).
pub fn reveal(depths: &[usize], folded: &mut BTreeSet<usize>, i: usize) -> bool {
    let mut changed = false;
    let mut at = parent_of(depths, i);
    while let Some(p) = at {
        changed |= folded.remove(&p);
        at = parent_of(depths, p);
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    // 0 Part I
    //   1 Ch 1
    //     2 Sec a
    //     3 Sec b
    //   4 Ch 2
    // 5 Part II
    //   6 Ch 3
    const D: [usize; 7] = [0, 1, 2, 2, 1, 0, 1];

    #[test]
    fn parents_are_the_entries_with_deeper_entries_after_them() {
        let p: Vec<usize> = (0..D.len()).filter(|&i| is_parent(&D, i)).collect();
        assert_eq!(p, vec![0, 1, 5]);
        assert!(!is_parent(&D, 99));
        assert!(!is_parent(&[], 0));
    }

    #[test]
    fn parent_of_finds_the_shallower_entry_before() {
        assert_eq!(parent_of(&D, 3), Some(1));
        assert_eq!(parent_of(&D, 4), Some(0));
        assert_eq!(parent_of(&D, 6), Some(5));
        assert_eq!(parent_of(&D, 0), None);
        assert_eq!(parent_of(&D, 5), None);
    }

    #[test]
    fn nothing_folded_shows_everything() {
        assert_eq!(visible(&D, &BTreeSet::new()), vec![0, 1, 2, 3, 4, 5, 6]);
        assert!(visible(&[], &BTreeSet::new()).is_empty());
    }

    #[test]
    fn a_folded_parent_hides_its_whole_subtree() {
        let f: BTreeSet<usize> = [1].into();
        assert_eq!(visible(&D, &f), vec![0, 1, 4, 5, 6]);
        let f: BTreeSet<usize> = [0].into();
        assert_eq!(visible(&D, &f), vec![0, 5, 6]);
        // a fold inside a folded parent changes nothing until the outer one opens
        let f: BTreeSet<usize> = [0, 1].into();
        assert_eq!(visible(&D, &f), vec![0, 5, 6]);
        let f: BTreeSet<usize> = [3].into();
        assert_eq!(
            visible(&D, &f),
            vec![0, 1, 2, 3, 4, 5, 6],
            "a leaf cannot fold"
        );
    }

    #[test]
    fn fold_all_folds_only_parents_and_shows_the_top_level() {
        let f = fold_all(&D);
        assert_eq!(f, [0, 1, 5].into());
        assert_eq!(visible(&D, &f), vec![0, 5]);
    }

    #[test]
    fn a_hidden_entry_shows_as_its_nearest_visible_ancestor() {
        let f: BTreeSet<usize> = [0].into();
        let v = visible(&D, &f);
        assert_eq!(position_of(&D, &v, 5), Some(1));
        assert_eq!(position_of(&D, &v, 3), Some(0), "inside Part I");
        let f: BTreeSet<usize> = [1].into();
        let v = visible(&D, &f);
        assert_eq!(position_of(&D, &v, 2), Some(1), "Sec a shows as Ch 1");
        assert_eq!(position_of(&D, &v, 4), Some(2));
        assert_eq!(position_of(&D, &[], 0), None);
    }

    #[test]
    fn revealing_unfolds_the_ancestors() {
        let mut f = fold_all(&D);
        assert!(reveal(&D, &mut f, 3));
        assert_eq!(f, [5].into());
        assert!(!reveal(&D, &mut f, 3), "nothing more to open");
        let v = visible(&D, &f);
        assert!(v.contains(&3));
    }

    #[test]
    fn odd_depth_jumps_do_not_break_it() {
        // a book whose contents jump from 0 to 3 and back
        let d = [0usize, 3, 3, 1, 0];
        let f = fold_all(&d);
        assert_eq!(visible(&d, &f), vec![0, 4]);
        let mut o = f.clone();
        reveal(&d, &mut o, 2);
        assert!(visible(&d, &o).contains(&2));
    }
}
