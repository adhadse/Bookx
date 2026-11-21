//! Taking a highlight (with its note) or a bookmark away and bringing it back (XC-1, RDR-R12): the
//! reader removes at once, shows a toast with Undo, and Undo puts the item back where it was.

use crate::core::library::{Bookmark, Highlight};

/// What was removed, and where it sat in the list.
#[derive(Debug, Clone, PartialEq)]
pub enum Removed {
    Highlight(usize, Highlight),
    Bookmark(usize, Bookmark),
}

fn take_where<T>(v: &mut Vec<T>, pred: impl Fn(&T) -> bool) -> Option<(usize, T)> {
    let i = v.iter().position(pred)?;
    Some((i, v.remove(i)))
}

fn put_back<T>(v: &mut Vec<T>, at: usize, item: T) {
    let at = at.min(v.len());
    v.insert(at, item);
}

/// Removes the highlight `id` (its note goes with it). `None` when it is not there.
pub fn take_highlight(list: &mut Vec<Highlight>, id: &str) -> Option<Removed> {
    take_where(list, |h| h.id == id).map(|(i, h)| Removed::Highlight(i, h))
}

/// Removes the bookmark `id`. `None` when it is not there.
pub fn take_bookmark(list: &mut Vec<Bookmark>, id: &str) -> Option<Removed> {
    take_where(list, |b| b.id == id).map(|(i, b)| Removed::Bookmark(i, b))
}

/// Puts a removed item back. Does nothing when an item with that id exists again (a second Undo).
pub fn restore(highlights: &mut Vec<Highlight>, bookmarks: &mut Vec<Bookmark>, r: Removed) -> bool {
    match r {
        Removed::Highlight(at, h) => {
            if highlights.iter().any(|x| x.id == h.id) {
                return false;
            }
            put_back(highlights, at, h);
            true
        }
        Removed::Bookmark(at, b) => {
            if bookmarks.iter().any(|x| x.id == b.id) {
                return false;
            }
            put_back(bookmarks, at, b);
            true
        }
    }
}

/// The text of the toast.
pub fn message(r: &Removed) -> String {
    match r {
        Removed::Highlight(..) => crate::i18n::tr("Highlight removed"),
        Removed::Bookmark(..) => crate::i18n::tr("Bookmark removed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn hl(id: &str, note: &str) -> Highlight {
        Highlight {
            id: id.into(),
            spine: 0,
            start: 0,
            end: 5,
            color: "yellow".into(),
            style: String::new(),
            note: note.into(),
            text: "words".into(),
            created: Utc::now(),
        }
    }

    fn bm(id: &str) -> Bookmark {
        Bookmark {
            id: id.into(),
            spine: 1,
            frac: 0.5,
            label: "Page 3".into(),
            created: Utc::now(),
        }
    }

    #[test]
    fn a_removed_highlight_comes_back_with_its_note_in_the_same_place() {
        let mut list = vec![hl("a", ""), hl("b", "my note"), hl("c", "")];
        let r = take_highlight(&mut list, "b").unwrap();
        assert_eq!(list.len(), 2);
        let mut marks = vec![];
        assert!(restore(&mut list, &mut marks, r));
        assert_eq!(list[1].id, "b");
        assert_eq!(list[1].note, "my note");
    }

    #[test]
    fn a_removed_bookmark_comes_back() {
        let mut marks = vec![bm("x"), bm("y")];
        let r = take_bookmark(&mut marks, "x").unwrap();
        assert_eq!(marks.len(), 1);
        let mut hls = vec![];
        assert!(restore(&mut hls, &mut marks, r));
        assert_eq!(marks[0].id, "x");
    }

    #[test]
    fn undo_twice_does_not_duplicate() {
        let mut list = vec![hl("a", "")];
        let r = take_highlight(&mut list, "a").unwrap();
        let mut marks = vec![];
        assert!(restore(&mut list, &mut marks, r.clone()));
        assert!(!restore(&mut list, &mut marks, r));
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn an_unknown_id_removes_nothing() {
        let mut list = vec![hl("a", "")];
        assert!(take_highlight(&mut list, "zzz").is_none());
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn a_short_list_clamps_the_place() {
        let mut list = vec![hl("a", ""), hl("b", ""), hl("c", "")];
        let r = take_highlight(&mut list, "c").unwrap();
        list.clear();
        let mut marks = vec![];
        assert!(restore(&mut list, &mut marks, r));
        assert_eq!(list.len(), 1);
    }
}
