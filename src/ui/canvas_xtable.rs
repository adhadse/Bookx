//! Fast x positions of byte offsets in a one-line layout, for the Knuth-Plass planner.
//!
//! `Layout::index_to_pos` walks the text from its start (UTF-8 offset to character offset) on every
//! call, so a planner that asks for the position of every word edge of a long paragraph is quadratic.
//! The table is built once by one pass over the clusters and answers from a sorted vector; offsets that
//! are not at a cluster start (inside a ligature) and right-to-left text still go to Pango.

/// Whether `text` has characters of a right-to-left script (those need Pango's bidi positions).
pub fn has_rtl(text: &str) -> bool {
    text.chars().any(|c| {
        matches!(c as u32,
            0x0590..=0x08FF | 0xFB1D..=0xFDFF | 0xFE70..=0xFEFF | 0x10800..=0x10FFF | 0x1E800..=0x1EFFF)
    })
}

/// Sorted (byte offset, x) pairs of the starts of clusters.
pub struct XTable {
    at: Vec<(usize, f64)>,
}

impl XTable {
    /// `at` need not be sorted; the first entry for an offset wins.
    pub fn new(mut at: Vec<(usize, f64)>) -> Self {
        at.sort_by_key(|(b, _)| *b);
        at.dedup_by_key(|(b, _)| *b);
        XTable { at }
    }

    pub fn get(&self, byte: usize) -> Option<f64> {
        self.at
            .binary_search_by_key(&byte, |(b, _)| *b)
            .ok()
            .map(|i| self.at[i].1)
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.at.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_by_cluster_start() {
        let t = XTable::new(vec![(4, 40.0), (0, 0.0), (2, 20.0), (2, 99.0)]);
        assert_eq!(t.len(), 3);
        assert_eq!(t.get(0), Some(0.0));
        assert_eq!(t.get(2), Some(20.0));
        assert_eq!(t.get(3), None);
        assert_eq!(t.get(4), Some(40.0));
    }

    #[test]
    fn rtl_detection() {
        assert!(!has_rtl("Hello wörld 日本語"));
        assert!(has_rtl("abc שלום"));
        assert!(has_rtl("مرحبا"));
    }
}
