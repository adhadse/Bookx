//! Merging books as formats of one book, with the state that lives in the catalogue store
//! (collections, reading status). The file handling is `bookx_core::merge`.

use anyhow::Result;
use bookx_core::library::Library;

use crate::{ReadingStatus, SqliteStore};

/// Moves what the store knows about `others` to `primary` before they are merged away:
/// collections, the reading status, the furthest progress and the last read date. The merged
/// books keep their own records in the trash, so undo loses nothing.
pub fn carry_state(lib: &mut Library, store: &SqliteStore, primary: u64, others: &[u64]) {
    let mut progress = lib
        .get(primary)
        .map(|b| (b.progress, b.position, b.last_read));
    for o in others {
        for c in store.collections_of(*o) {
            let _ = store.add_to_collection(c, primary);
        }
        // a book never touched is "new": any other status of a merged copy wins over it
        if store.get_status(primary) == Some(ReadingStatus::New) {
            if let Some(s) = store.get_status(*o) {
                let _ = store.set_status(primary, s);
            }
        }
        if let (Some(cur), Some(b)) = (progress.as_mut(), lib.get(*o)) {
            if b.progress > cur.0 {
                *cur = (b.progress, b.position, b.last_read.max(cur.2));
            } else {
                cur.2 = cur.2.max(b.last_read);
            }
        }
    }
    if let (Some((p, pos, lr)), Some(b)) = (progress, lib.get_mut(primary)) {
        b.progress = p;
        b.position = pos;
        b.last_read = lr;
    }
}

/// Merges `others` into `primary` as formats of one book, keeping collections and status.
/// Returns the number of books merged. A merge that is refused changes nothing.
pub fn merge(
    lib: &mut Library,
    store: &SqliteStore,
    primary: u64,
    others: &[u64],
) -> Result<usize> {
    let plan = lib.plan_merge(primary, others)?;
    let ids: Vec<u64> = plan.mergeable().iter().map(|i| i.id).collect();
    if ids.is_empty() {
        anyhow::bail!("nothing to merge: the books have no format the first one lacks");
    }
    carry_state(lib, store, primary, &ids);
    lib.merge_formats(primary, &ids)
}
