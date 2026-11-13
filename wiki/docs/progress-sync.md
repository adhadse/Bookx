# Sync your reading position (KOSync)

Bookx can keep your place in step with KOReader and other apps through a **KOReader progress-sync (“KOSync”)
server**: Bookx’s own server (see [Share Library](devices-and-sharing.md#share-library)), a self-hosted kosync, or
the public one at `sync.koreader.rocks`. Only the **position** is synced, not highlights or notes (for those see
[Import Highlights](annotations.md) and [Devices and sharing](devices-and-sharing.md)).

## Set it up

*Preferences → Sync → **Sync reading position (KOSync)***:

1. **Server address**: for Bookx’s own server the address KOReader shows in Share Library, ending in `/kosync`
   (for example `http://192.168.1.5:8080/kosync`); for the public server `https://sync.koreader.rocks`.
2. **User name** and **Password**. The password is kept in the system keyring, like the Share and e-mail passwords (the settings file only when there is no keyring).
3. **Register** creates the account on the server (and logs in); **Log in** checks an account that already exists,
   for example one made in KOReader. The result is shown next to the buttons.
4. **Match books by** must be the same as in KOReader (*Tools → Progress sync → Document matching method*):
   - **Partial MD5 of the file** (the default in both): books match when the files are byte-for-byte the same.
     Renaming is fine, converting or editing the book changes the id.
   - **File name**: books match when the file names are the same.
5. **This device is called** is the name the other devices show (“Continue from *Bookx on my laptop*?”).
6. Switch **Sync the reading position** on.

## What happens

- **Opening a book** asks the server where it is up to. If another device read further than you are here by more
  than 1 %, Bookx asks **“Continue from Kobo Libra at 63%?”** with **Stay Here** and **Continue**. Nothing moves
  unless you choose Continue, and a position *behind* yours is never offered.
- **Reading** pushes your position a few seconds after you stop turning pages, and **closing the book** pushes at once.
- Until you have answered the question, a push that would take the server **backwards** (another device is further
  along) is held back instead of overwriting it.
- The reading position shown in your library follows what you read here; it is not changed by a position you did
  not accept.

## Limits

- **The position is a percentage.** KOReader describes a place as a point in *its* layout of the book; Bookx cannot
  place that, so it turns the percentage into a chapter and a place in it with its own chapter weights. Expect to
  land within a page or two of where the other device was, further off in books with large images.
- **Bookx tells KOReader the start of the chapter** you are in (plus the exact percentage), so KOReader opens at the
  beginning of that chapter, not at the same line.
- Document ids are only equal for the same file (or file name). A converted or edited copy is a different book to
  the server.
- **Offline is fine.** If the server cannot be reached, the position is kept (one entry per book, the newest) and sent
  later: when Bookx starts, every minute, and as soon as the network comes back. The pause between tries grows from
  30 seconds to an hour. A server that refuses your login drops the entry instead of trying for ever. Opening a book
  still needs the network to ask where you were.
- Only the position is synced. Use [Import Highlights](annotations.md) for highlights and notes.
