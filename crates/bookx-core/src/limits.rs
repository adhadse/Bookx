//! The size and depth limits that guard against hostile input (security roadmap P1.3), in one place.
//!
//! Every limit has a name, a value and a reason. Code that reads untrusted bytes takes its limit from here, so a
//! reviewer can see all of them at once and a test can check that they are consistent ([`all`]). Limits that
//! belong to one format only (the PDF outline depth, the SMIL clock) stay next to their parser and are listed
//! in `docs/security.md` §4.2.
//!
//! Migrated so far: `Archive::MAX_MEMBER` / `MAX_TOTAL`, `util::MAX_HTML_DEPTH`, `util::MAX_ATTRS_PER_ELEMENT`,
//! `util::MAX_LAUNCH_URL` and the query pattern length. Still local (to migrate): `bookx-editing` (cover bytes
//! and pixels, SMIL bytes, scanned font bytes), `bookx-pdf` (outline entries and depth), the plugin host.

/// Largest size one member of a zip (EPUB, CBZ, DOCX …) may expand to.
pub const ARCHIVE_MEMBER_BYTES: u64 = 512 << 20;
/// Largest total size of all members of one archive: the zip-bomb ceiling.
pub const ARCHIVE_TOTAL_BYTES: u64 = 2 << 30;
/// Deepest element nesting accepted in HTML from a book, feed or web page; html5ever and every recursive walk
/// over its tree overflow the stack long before 100 000 levels.
pub const HTML_DEPTH: usize = 2000;
/// Most attributes one element may carry; the XML parser compares each new one with the earlier ones.
pub const ATTRS_PER_ELEMENT: usize = 5000;
/// The longest link Bookx hands to the desktop.
pub const LAUNCH_URL_BYTES: usize = 8192;
/// Longest pattern a library search accepts; longer ones match nothing.
pub const QUERY_PATTERN_CHARS: usize = 256;

/// A named limit, for documentation and the consistency test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Limit {
    pub name: &'static str,
    pub value: u64,
    pub unit: &'static str,
}

/// Every limit of this module.
pub fn all() -> Vec<Limit> {
    let l = |name, value: u64, unit| Limit { name, value, unit };
    vec![
        l("ARCHIVE_MEMBER_BYTES", ARCHIVE_MEMBER_BYTES, "bytes"),
        l("ARCHIVE_TOTAL_BYTES", ARCHIVE_TOTAL_BYTES, "bytes"),
        l("HTML_DEPTH", HTML_DEPTH as u64, "levels"),
        l("ATTRS_PER_ELEMENT", ATTRS_PER_ELEMENT as u64, "attributes"),
        l("LAUNCH_URL_BYTES", LAUNCH_URL_BYTES as u64, "bytes"),
        l(
            "QUERY_PATTERN_CHARS",
            QUERY_PATTERN_CHARS as u64,
            "characters",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn limits_are_positive_and_one_member_fits_in_the_total() {
        for l in all() {
            assert!(l.value > 0, "{}", l.name);
        }
        const { assert!(ARCHIVE_MEMBER_BYTES <= ARCHIVE_TOTAL_BYTES) };
    }

    #[test]
    fn the_old_names_still_point_here() {
        assert_eq!(crate::archive::Archive::MAX_MEMBER, ARCHIVE_MEMBER_BYTES);
        assert_eq!(crate::archive::Archive::MAX_TOTAL, ARCHIVE_TOTAL_BYTES);
        assert_eq!(crate::util::MAX_HTML_DEPTH, HTML_DEPTH);
        assert_eq!(crate::util::MAX_ATTRS_PER_ELEMENT, ATTRS_PER_ELEMENT);
        assert_eq!(crate::util::MAX_LAUNCH_URL, LAUNCH_URL_BYTES);
    }

    #[test]
    fn names_are_unique() {
        let mut n: Vec<_> = all().iter().map(|l| l.name).collect();
        n.sort();
        n.dedup();
        assert_eq!(n.len(), all().len());
    }
}
