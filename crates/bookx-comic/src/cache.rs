//! A cache of decoded pages with a budget in bytes. The least recently used pages go first, and
//! pages the reader asks to keep (the one on screen and its neighbours) are never evicted.

use std::collections::HashMap;

pub struct PageCache<V> {
    budget: usize,
    used: usize,
    tick: u64,
    items: HashMap<usize, (V, usize, u64)>,
}

impl<V> PageCache<V> {
    pub fn new(budget: usize) -> Self {
        PageCache {
            budget,
            used: 0,
            tick: 0,
            items: HashMap::new(),
        }
    }

    pub fn used(&self) -> usize {
        self.used
    }
    pub fn len(&self) -> usize {
        self.items.len()
    }
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
    pub fn contains(&self, page: usize) -> bool {
        self.items.contains_key(&page)
    }

    /// The page, marked as just used.
    pub fn get(&mut self, page: usize) -> Option<&V> {
        self.tick += 1;
        let t = self.tick;
        self.items.get_mut(&page).map(|e| {
            e.2 = t;
            &e.0
        })
    }

    /// Without marking it as used.
    pub fn peek(&self, page: usize) -> Option<&V> {
        self.items.get(&page).map(|e| &e.0)
    }

    pub fn insert(&mut self, page: usize, v: V, bytes: usize) {
        self.tick += 1;
        if let Some(old) = self.items.insert(page, (v, bytes, self.tick)) {
            self.used -= old.1;
        }
        self.used += bytes;
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.used = 0;
    }

    /// Drops the least recently used pages until the budget holds. Pages in `keep` stay even when
    /// that goes over the budget.
    pub fn trim(&mut self, keep: &[usize]) -> Vec<usize> {
        let mut dropped = Vec::new();
        while self.used > self.budget {
            let victim = self
                .items
                .iter()
                .filter(|(p, _)| !keep.contains(p))
                .min_by_key(|(_, e)| e.2)
                .map(|(p, _)| *p);
            let Some(p) = victim else { break };
            if let Some(e) = self.items.remove(&p) {
                self.used -= e.1;
            }
            dropped.push(p);
        }
        dropped
    }
}

/// The pages to have ready: the visible ones first, then the next ones in the reading direction
/// (`ahead`), then one behind. Pages outside `0..count` are left out.
pub fn preload_order(visible: &[usize], count: usize, forward: bool, ahead: usize) -> Vec<usize> {
    let mut out: Vec<usize> = visible.iter().copied().filter(|p| *p < count).collect();
    let (lo, hi) = match (visible.iter().min(), visible.iter().max()) {
        (Some(a), Some(b)) => (*a, *b),
        _ => return out,
    };
    let next = |k: usize| -> Option<usize> {
        if forward {
            hi.checked_add(k).filter(|p| *p < count)
        } else {
            lo.checked_sub(k)
        }
    };
    let prev = |k: usize| -> Option<usize> {
        if forward {
            lo.checked_sub(k)
        } else {
            hi.checked_add(k)
        }
        .filter(|p| *p < count)
    };
    for k in 1..=ahead {
        out.extend(next(k));
        if k == 1 {
            out.extend(prev(1));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_least_recently_used_over_budget() {
        let mut c = PageCache::new(250);
        c.insert(0, "a", 100);
        c.insert(1, "b", 100);
        assert!(c.get(0).is_some());
        c.insert(2, "c", 100);
        let dropped = c.trim(&[]);
        assert_eq!(dropped, vec![1]);
        assert!(c.contains(0) && c.contains(2) && !c.contains(1));
        assert_eq!(c.used(), 200);
    }

    #[test]
    fn kept_pages_survive_even_over_budget() {
        let mut c = PageCache::new(50);
        c.insert(0, 0, 100);
        c.insert(1, 1, 100);
        assert!(c.trim(&[0, 1]).is_empty());
        assert_eq!(c.len(), 2);
        assert_eq!(c.trim(&[1]), vec![0]);
    }

    #[test]
    fn replacing_a_page_updates_the_total() {
        let mut c = PageCache::new(1000);
        c.insert(3, 'x', 400);
        c.insert(3, 'y', 100);
        assert_eq!(c.used(), 100);
        assert_eq!(c.peek(3), Some(&'y'));
        c.clear();
        assert!(c.is_empty() && c.used() == 0);
    }

    #[test]
    fn preload_goes_in_the_reading_direction() {
        assert_eq!(preload_order(&[4], 10, true, 3), vec![4, 5, 3, 6, 7]);
        assert_eq!(preload_order(&[4], 10, false, 3), vec![4, 3, 5, 2, 1]);
        assert_eq!(preload_order(&[0], 10, true, 2), vec![0, 1, 2]);
        assert_eq!(preload_order(&[9], 10, true, 2), vec![9, 8]);
        assert_eq!(preload_order(&[4, 5], 10, true, 2), vec![4, 5, 6, 3, 7]);
        assert!(preload_order(&[], 10, true, 3).is_empty());
        assert_eq!(preload_order(&[12], 10, true, 3), Vec::<usize>::new());
    }
}
