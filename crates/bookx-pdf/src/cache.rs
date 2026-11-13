//! A least-recently-used cache with a byte budget, for rendered pages.

use std::collections::HashMap;
use std::hash::Hash;

pub struct PageCache<K, V> {
    budget: usize,
    used: usize,
    tick: u64,
    map: HashMap<K, (V, usize, u64)>,
}

impl<K: Hash + Eq + Clone, V> PageCache<K, V> {
    pub fn new(budget_bytes: usize) -> Self {
        PageCache {
            budget: budget_bytes,
            used: 0,
            tick: 0,
            map: HashMap::new(),
        }
    }

    pub fn get(&mut self, k: &K) -> Option<&V> {
        self.tick += 1;
        let t = self.tick;
        self.map.get_mut(k).map(|e| {
            e.2 = t;
            &e.0
        })
    }

    pub fn contains(&self, k: &K) -> bool {
        self.map.contains_key(k)
    }

    /// Stores `v` (`bytes` large); the least recently used entries go until the budget holds.
    /// The new entry is always kept, even when it is larger than the budget.
    pub fn insert(&mut self, k: K, v: V, bytes: usize) {
        self.tick += 1;
        if let Some((_, old, _)) = self.map.insert(k.clone(), (v, bytes, self.tick)) {
            self.used -= old;
        }
        self.used += bytes;
        while self.used > self.budget && self.map.len() > 1 {
            let oldest = self
                .map
                .iter()
                .filter(|(key, _)| **key != k)
                .min_by_key(|(_, e)| e.2)
                .map(|(key, _)| key.clone());
            match oldest {
                Some(o) => {
                    if let Some((_, b, _)) = self.map.remove(&o) {
                        self.used -= b;
                    }
                }
                None => break,
            }
        }
    }

    /// Drops every entry for which `keep` is false.
    pub fn retain(&mut self, mut keep: impl FnMut(&K) -> bool) {
        let used = &mut self.used;
        self.map.retain(|k, e| {
            let k = keep(k);
            if !k {
                *used -= e.1;
            }
            k
        });
    }

    pub fn clear(&mut self) {
        self.map.clear();
        self.used = 0;
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn used(&self) -> usize {
        self.used
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evicts_the_least_recently_used_over_budget() {
        let mut c: PageCache<u32, &str> = PageCache::new(100);
        c.insert(1, "a", 40);
        c.insert(2, "b", 40);
        assert_eq!(c.get(&1), Some(&"a")); // 1 is now fresher than 2
        c.insert(3, "c", 40);
        assert!(c.contains(&1) && c.contains(&3));
        assert!(!c.contains(&2));
        assert_eq!(c.used(), 80);
    }

    #[test]
    fn replacing_a_key_does_not_leak_bytes() {
        let mut c: PageCache<u32, u8> = PageCache::new(100);
        c.insert(1, 1, 60);
        c.insert(1, 2, 30);
        assert_eq!(c.used(), 30);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get(&1), Some(&2));
    }

    #[test]
    fn an_oversized_entry_is_kept_alone() {
        let mut c: PageCache<u32, u8> = PageCache::new(10);
        c.insert(1, 1, 5);
        c.insert(2, 2, 50);
        assert!(c.contains(&2));
        assert!(!c.contains(&1));
        assert_eq!(c.len(), 1);
    }

    #[test]
    fn retain_and_clear_keep_the_count_right() {
        let mut c: PageCache<u32, u8> = PageCache::new(1000);
        for i in 0..10 {
            c.insert(i, 0, 10);
        }
        c.retain(|k| k % 2 == 0);
        assert_eq!(c.len(), 5);
        assert_eq!(c.used(), 50);
        c.clear();
        assert!(c.is_empty());
        assert_eq!(c.used(), 0);
    }
}
