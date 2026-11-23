# Sync rules (internal)

Owner decision, issue #39 (2026-11-23).

## The rule

Every sync (KOReader / KOSync / Bookx sync over Wi-Fi, the cable sync of Kobo and KOReader state, the
Kobo, Kindle and PocketBook readers, the OPDS server) moves **reading position, highlights, notes, bookmarks and
similar metadata**. It never moves a book file, in either direction, and **nothing that disappears on a device
deletes or hides a library book, its files, or what was made in Bookx**: a book taken off the reader, a wiped
device, a deleted KOReader `.sdr` folder or notes file, a device that reports no books.

Books reach the library only through *Add Books* (the user's act). Books reach a device only through
*Send to Device*. The only deletion on a device is *Remove from Device*, chosen and confirmed by the user, and it
deletes on the device only (`devfs::remove_book` refuses anything outside the mount).

## What was audited (2026-11-23)

| Path | Reads from the device | Can it remove a library book/file? | Test |
|---|---|---|---|
| KOSync progress (`kosync.rs`, `kosync_client.rs`) | position | no (no library access; own store file only) | `bookx-serve/tests/sync_rule.rs` |
| Annotation sync server (`annotation_sync.rs` in serve) | highlights, notes, bookmarks, tombstones | no | same |
| Annotation merge into the library (`src/annotation_sync.rs`, `merge_incoming(&mut [Book])`) | same | no: it takes a slice of books, it cannot remove one. Highlights: see below | `a_device_that_deletes_everything_keeps_every_book_and_everything_made_in_bookx` |
| Cable sync (`src/device_sync.rs`, `plan` + `apply_to_library`) | `.sdr`, Kobo database | no: the plan has no removals; a missing book plans nothing | `a_device_without_books_or_without_this_book_plans_nothing_for_the_library` |
| Bookmark / look sync | marks, colours | no | `src/sync_rule.rs` scan |
| Kobo shelves (`kobo.rs`) | database copy | only adds to a shelf of the device; the only delete is its own temp copy | scan |
| Remove from Device (`devices.rs`, `devfs::remove_book`) | n/a | deletes on the **device** only, user-chosen and confirmed | `devfs` tests |

`src/sync_rule.rs` and `crates/bookx-serve/tests/sync_rule.rs` scan the sync code for any call that removes a
file or a book: a new one fails the build and has to be argued with the owner.

## The one exception to flag (annotation sync, by design)

A highlight that **came from the device** (`made_here`) and was then deleted on the device is deleted in Bookx
too, but only if its note was not changed in Bookx and the deletion is not older than the last change. A highlight
made in Bookx, or edited in Bookx, is never removed by a device (tested). The owner's rule is about **books**; this
per-highlight behaviour is documented on the wiki (koreader-sync.md, "What happens") and was kept. If the owner
wants "a device never deletes anything in the library", set `rep.removed` paths to keep (one place:
`merge_incoming`, state `Removed`).
