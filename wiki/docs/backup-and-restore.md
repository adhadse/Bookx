# Backup and restore

A **backup** in Bookx holds your **books**: the book files, their covers and the catalogue with everything you
have added to it (highlights, notes, bookmarks, reading progress, collections, tags). Open the main menu (the ☰ button
in the library's header bar), choose **Library Health** and pick **Backups** in its sidebar: that page shows what is
included, where backups go and lets you restore them.

!!! important "What a backup contains"
    | Included | Not included |
    |---|---|
    | Your book files (EPUB, PDF, comics, audiobooks and the rest) and their covers | Configuration (Preferences), plugins and saved passwords |
    | The catalogue: titles, authors, tags, series, collections and identifiers | The trash and the `.bookx/` state folder |
    | Reading progress, bookmarks, highlights and notes, and the [history of deleted and changed highlights](annotations.md#history) | Copies on your devices and caches |

    Each place can leave out the book files, the covers, or files above a size you choose (see
    [Choosing what a place copies](#choosing-what-a-place-copies)). Whatever is left out is listed, never lost silently.

### Only what is new is copied

The **first backup to a place copies everything**. After that Bookx copies **only what is new or changed**: a book you
added, a file you edited, the catalogue. Everything else is already in the place and is not sent again.

- **Renaming or moving a book costs nothing.** A book is recognised by its content, not its name or folder.
- **The same book twice is stored once.**
- **Nothing new, nothing copied.** A backup with nothing to do says *Nothing new to copy* and writes nothing.
- **Notes and highlights are saved within minutes.** Between backups Bookx writes a very small record of the
  highlights, notes, bookmarks and progress that changed, about every five minutes while it is open and when it closes.
  If the computer dies before the next backup, a restore still brings those back.
- **Before a copy starts** each place shows how much it will take: *The first backup copies everything: about 3.2 GB in
  412 files.* later *12 MB of new or changed files (2 files) to copy; everything else is already there.*
- **Every file is read back and checked** after it is written. A file that does not read back correctly is removed and
  the place shows *Failed*, so a full disk or a flaky share cannot silently give you an unusable backup.

!!! note "Backups are one-way copies, not sync"
    A backup is a copy at one moment (plus the small notes records). Nothing flows back by itself, and two computers
    cannot use one library through backups. Bookx does not sync a library between computers. See
    [Which sync do I need?](devices-and-sharing.md#which-sync-do-i-need) for what Bookx can sync (reading position with
    KOReader, a book list with a device).

### How big is a backup?

About the size of your library once. Later backups add only what you add. Many snapshots of the same library cost
almost nothing extra, because each snapshot is a list of files, not another copy of them. Encrypted places are a little
bigger (a few bytes per file). Bookx checks the free space of a place before it starts and says *Not enough free space*
instead of failing half way.

## Where backups go

Further down the Backups page of Library Health, the **Where backups go** list shows every place with its **path or address**,
**what it copies** (for example *Books, covers and catalogue*), how many **backups** and books it holds, their **size**, the
**time of the last one**, what the **next backup will copy**, and a **status**:

| Status | Meaning |
|---|---|
| OK | The last copy worked (and was checked, see below). |
| Unreachable | The drive is not plugged in, the share is not mounted, the server did not answer. The reason is shown. Bookx tries again next time. |
| Failed | The place answered but the copy did not work: wrong password, full, no permission. The reason is shown. |
| Not used yet / Switched off | A new place, or one you switched off with its switch. |

**On this computer** is always there: the library folder's `.bookx-backups` folder, which holds quick snapshots of
the catalogue only (so you can undo a bad import in a moment). Your books are copied to the **other places** you add with **Add a place**;
every backup goes to all of them that are switched on. A place that is down does not stop the others, and each
place shows its own problem (full, unreachable, wrong password).

### Kinds of places

| Place | How | Supported |
|---|---|---|
| Folder on this computer | *Folder or external drive* → Choose | Yes |
| External drive, USB disk, SD card | The same; mounted drives are recognised and labelled | Yes |
| Network share (SMB/Windows, NFS, SFTP, NAS) | *Network share*: type `smb://nas/books` (or `sftp://…`, `nfs://…`) and *Connect*, or pick a share already open in Files. GNOME mounts it, Bookx copies into the mounted folder | Yes, needs GNOME’s GVfs (normal on GNOME). The mount is not kept after logging out; Bookx asks again when you connect |
| Nextcloud, ownCloud, other WebDAV | *WebDAV*: address, user, password, folder. Bookx talks WebDAV itself | Yes. Tested against a local stand-in server, not yet against a real Nextcloud |
| Google Drive, OneDrive | Add the account in **GNOME Settings → Online Accounts** and switch on *Files*. It then shows up under *Cloud drive* | Yes, as a file copy into the mounted drive. Uploads are as slow as GNOME’s mount; for very large catalogues prefer WebDAV or a folder your cloud client syncs |
| Nextcloud through Online Accounts | The same route as above, or use WebDAV | Yes |
| iCloud Drive | Apple offers no way in from Linux. If a desktop sync client keeps an *iCloud Drive* folder up to date on your computer, choose that folder | Only that way. Otherwise **not supported** |
| Dropbox and similar | Choose the folder their app syncs | Yes, as a folder |

Bookx does not store Google or Microsoft logins: GNOME Online Accounts does, and Bookx only sees a folder.

### Several places at once

Add as many as you like, for example a USB disk (plain), a NAS (plain) and a Nextcloud (encrypted). Each place has
its own **Automatic backups to keep here**. Manual backups (from **Back up now**) are never removed. Only **one
computer** should back up into a place: two computers backing up into the same folder is not supported (use a folder each).

### Choosing what a place copies

In **Add a Place** or **Edit Place**:

- **Include book files**: on by default. Off keeps only the catalogue with your notes, which is tiny.
- **Include covers**: on by default.
- **Largest file to copy, in MB**: `0` means no limit. Bigger files are left out and listed (for example a few huge
  audiobooks on a small drive).
- **Make the first copy automatically**: see [the first copy](#the-first-copy-waits-for-you).

Names that Windows cannot hold (such as `con.epub`) are left out and reported, so a copy on a Windows-formatted drive
stays restorable. The Backups page says *N files are left out* when that happens.

## Making backups

<figure markdown="1">
![The Backups tab of Library Health](assets/library-health-backups-light.png#only-light)
![The Backups tab of Library Health](assets/library-health-backups-dark.png#only-dark)
<figcaption markdown="1">The Backups page of Library Health: what a backup contains, the Back Up button and the places backups are copied to, each with what it holds and what the next backup will copy.</figcaption>
</figure>


- **Automatically on this computer**: the **Automatic backups** switch at the top of the page takes a quick snapshot of
  the catalogue when Bookx starts and closes, if something changed. **Snapshots to keep** sets how many are kept.
- **To other places**: *Copy when Bookx closes* and *Copy once a day while Bookx is open* decide when your places get a
  backup. Only what is new or changed is copied. When Bookx closes it waits at most 20 seconds for a place that does
  not answer. A copy that is cut short leaves no half-written file and is finished next time.
- **Back up now**: press **Back Up** on that row. It saves a snapshot here and copies to every switched-on place. A
  progress bar names the step (*Looking at your library*, *Finding what is new*, *Copying new files*, *Writing the
  snapshot*), shows files and bytes done and a time left, and **Cancel** stops it. The same job is listed in the Jobs
  panel of the library. When it ends, a message says what was copied (*Copied 3 new files (12 MB); 410 were already
  there.*), what was left out and which place failed.
- **One place only**: the **⋯** button on a place has **Back Up This Place**.

### The first copy waits for you

The first copy of a place can be large (your whole library), so Bookx never starts it behind your back. After an
update, or when you add a place, a row on the Backups page says *Include your books in backups* and how big the first
copy is. Nothing is copied until you either press **Back Up** or switch on **Make the first copy automatically** in the
place's settings. From the first successful copy on, *Copy when Bookx closes* and *Copy once a day* work for that place
without asking again.

### Checking and cleaning a place

Press **⋯** on a place:

- **Check Backup** looks that every snapshot opens and every file it needs is there with the right size.
- **Check Backup in Depth** also reads every file back and compares it. It reads everything, so it takes a while on a
  network place.
- **Remove Old Backups** removes the backups beyond *Automatic backups to keep here* (manual ones stay) and the files
  that only they needed, after you confirm.

If a check finds a missing or damaged file, press **Back Up**: it copies the missing files again. A deep check shows
how much it has read, how fast, and about how long is left.

- **Take Over This Place** is for the case below, where another computer is marked as writing to the place.

## Encrypting copies

When you add or edit a place, switch on **Encrypt the copies** (it is on by default for WebDAV). The **Backup
passphrase** itself is one field in the *Copies to other places* group of the same Backups page. Then every file in the
place is encrypted: books, covers, catalogue and notes.

- files are encrypted with **ChaCha20-Poly1305**; the key is made from your passphrase with **Argon2id**, a
  deliberately slow function, so a stolen place cannot be guessed at quickly;
- whoever runs the place sees that a backup exists, how many files there are and roughly how big they are, but not
  file names, titles, authors, notes or contents. Two copies of the same book are still stored once;
- a changed byte, a swapped or cut-off file is detected when checking and restoring;
- the passphrase is kept in the system keyring, not in the settings file ([Passwords](#passwords)).

!!! warning "Do not lose the passphrase"
    There is no recovery. Without the passphrase an encrypted backup cannot be opened by anyone, including you. Write
    it down somewhere safe.

### Changing the passphrase

On the Backups page press **Change** on the **Change Backup Passphrase** row. Enter the current passphrase (leave it empty to
use the saved one), the new one twice (at least 8 characters) and press **Change**. Bookx re-encrypts every encrypted
place in place, with a progress bar and **Cancel**. It takes about as long as reading and writing the whole backup, so
longer on a network place. If it is stopped (Cancel, a lost connection, a power cut), nothing is lost: until it is
finished the backup cannot be used, and running **Change** again with the same two passphrases continues where it
stopped. The saved passphrase is replaced only when every place is done. Afterwards the old passphrase no longer opens
the backups; anybody who kept a copy of the place from before the change can still open that copy with the old one.

A plain (not encrypted) place holds your book files as they are, in a `Bookx Backup` folder, so even without Bookx you
can get a book out by hand; a `README.txt` there explains how. Anyone who can read the place can read them: Bookx shows a note under every
off-site place that is not encrypted.

## Restoring

<figure markdown="1">
![The Trash and History tab of Library Health](assets/library-health-trash-light.png#only-light)
![The Trash and History tab of Library Health](assets/library-health-trash-dark.png#only-dark)
<figcaption markdown="1">The Trash page of Library Health: removed books can be restored, and the history of changes can be undone.</figcaption>
</figure>


Removed books and undoing single changes are not part of backups: they are on the **Trash** page of Library Health
(History and Trash). To go back to an older state, use the Backups page, which has two lists:

- **Restore from a backup** shows the quick catalogue snapshots on this computer (the catalogue only).
- **Restore from another place**: pick a place; its backups are listed (date, number of books, files, size). Press
  **Restore** on one. Encrypted ones use the saved passphrase, or ask for it. *Restore from a file* takes a `.db` or
  `.bookxbackup` file you copied yourself. Older catalogue-only files that sit in a place are listed too, marked
  *catalogue only*.

**Restore** opens a window where you choose:

- **Restore into**: *This library* or *Another folder* (for example a new, empty library on another disk).
- **What to restore**: *Replace everything* (the books and the catalogue of the backup) or *Only the missing books*
  (files that are not there are put back; files and the catalogue that exist are never touched). If your library is
  empty, *Replace everything* is chosen for you.

Press **Preview** first. Bookx then says what would happen, for example *42 files to restore, 310 already present. The
catalogue is replaced by the one in the backup; the current one is saved first.* and warns about files that are newer
here than in the backup. **Restore** only works after a preview, so nothing is replaced without you having seen it.
While restoring a progress bar and **Cancel** are shown; files already restored are complete, a cancelled file leaves
nothing behind.

Bookx checks the backup for damage first and saves the **current** catalogue as a *Before a restore* snapshot, so a
restore can be undone by restoring that one. Notes written after the last full backup come back too, from the small notes
records.

### Restoring on a new computer

Install Bookx, open the main menu → *Library Health* → *Backups*, **Add a place** pointing at the same drive, share or
WebDAV folder (enter the same passphrase if it was encrypted), pick the backup and press **Restore**. This brings the
**books, covers and catalogue** back into your new library; you do not need to copy the library folder yourself. If you only have a `.bookxbackup` or `.db` file from an older version, *Restore from a file*
brings back the catalogue only; the book files then have to come from your own copy of the library folder.

## Command line

```bash
bookx-cli backup plan    --to /run/media/me/BACKUP                  # how big the next backup is, without copying
bookx-cli backup run     --to /run/media/me/BACKUP --manual         # books, covers, catalogue: only what is new
bookx-cli backup run     --to https://cloud.example.com/remote.php/dav/files/me/ --encrypt
bookx-cli backup journal --to /run/media/me/BACKUP                  # record changed notes only
bookx-cli backup list    --to /run/media/me/BACKUP                  # snapshots (--journal: notes records)
bookx-cli backup restore latest --to /run/media/me/BACKUP --into ~/Books --mode replace-all --dry-run
bookx-cli backup verify  --to /run/media/me/BACKUP --deep           # check every file
bookx-cli backup gc      --to /run/media/me/BACKUP                  # remove files no backup uses
```

The older whole-catalogue commands still work (`bookx-cli library backup --to …`, `library snapshots`, `library restore`)
and read the old `.db` and `.bookxbackup` files. Secrets are never taken from the command line: the WebDAV password comes from
`BOOKX_WEBDAV_PASSWORD` and the passphrase from `BOOKX_BACKUP_PASSPHRASE`. `smb://` addresses must be mounted first
(`gio mount smb://nas/books`) and the folder under `/run/user/<id>/gvfs/` given. See [Command line](command-line.md).

## Passwords

Bookx **never writes a password or token to the settings file or any other plain file**. This covers the OPDS server
and catalogue passwords, e-mail, reading position sync, WebDAV for devices and backups, the backup passphrase, the
Readwise token, the read-aloud server key, Wallabag credentials and the assistant access token. Where they go depends on
your system, and the line under each password field says which:

- **System keyring** (GNOME Keyring, KWallet, KeePassXC; on macOS the Keychain, on Windows the Credential Manager; in the
  Flatpak the secrets portal). The keyring is protected by your login (or keyring) password. The field says *Kept in the
  system keyring, not in the settings file.*
- **No keyring: Bookx's encrypted password store.** The first time you save a password, Bookx offers to **create a master
  password**. The passwords then go into one file, `secrets.vault`, in Bookx's settings folder (readable by you only),
  encrypted with ChaCha20-Poly1305 under a key made from your master password with Argon2id. Bookx asks for the master
  password once each time it starts (*Unlock Saved Passwords*) and never saves it anywhere. If you forget it, the
  stored passwords cannot be recovered: choose *Forgot Password* to delete the store and type them again. The field
  says *Kept in Bookx's encrypted password store.*
- **No keyring and no master password** (you chose *Don't Save* or *Not Now*): the password works until you close
  Bookx and is then forgotten. The field says *Not saved: it is forgotten when Bookx closes.* Nothing is written.

Passwords that older versions left in the settings file are removed the first time this version starts (Bookx says
*Saved passwords were removed from the settings file*). With a keyring they are moved into it; without one they stay in
memory for that session so you can save them in the encrypted store, and if you decline you will have to type them again.

What this does **not** protect against: a program running as you while the keyring is unlocked or Bookx is unlocked can
ask for the passwords like Bookx does, and so can someone using your unlocked computer. A short master password can be
guessed by someone who copies `secrets.vault`; use a long one. The **backup passphrase** (it encrypts your backups) and
the **KOReader device key** (stored on the e-reader, see [KOReader sync](koreader-sync.md)) are separate secrets.

## Limits

- **One computer writes to one backup.** While Bookx writes to a place it leaves a small note there. A second computer
  that finds the note stops with *this backup place is in use* and the name of the other computer; wait until it is
  done. If the other computer was switched off in the middle of a copy, use **⋯ → Take Over This Place** (a note that
  nobody renewed for 15 minutes is ignored by itself). Reading and restoring are always allowed. If a place suddenly looks older than what Bookx saw before (an old copy was put back over it),
  Bookx stops and says so instead of overwriting newer backups.
- **WebDAV has not been tried against a real Nextcloud or ownCloud**, only against a stand-in server. Some servers
  limit the size of one upload (for example 100 MB): a bigger book then fails with a clear message about the server's
  limit, and the other files are still copied. Many small files over a slow link take time.
- **Google Drive, OneDrive and other cloud mounts** have not been tried with real accounts; they work as folders through
  GNOME's mounts, which can be slow for the first copy. A folder your cloud client syncs, or WebDAV, is faster.
- **Names Windows cannot hold** (such as `con.epub` or names with control characters) are left out and listed.
- A file that changes while it is being copied is left out and taken at the next backup.
- Journal records carry highlights, notes, bookmarks, progress and your collections (shelves). Restoring brings back
  the shelves and what is on them; a shelf you deleted after the last full backup comes back too.
- Big files to WebDAV go up in pieces and an interrupted upload continues, but only when the server accepts partial
  uploads and the place is not encrypted. Most servers (Nextcloud included) do not, and then each file is one upload; an
  encrypted file always starts again.
- Notes of a book are restored onto a book the restored catalogue has: a book that was added after the last full backup
  comes back with the next one.
- The copy is verified by reading it back, which doubles the traffic of the first backup to a network place.

## Recommended setup

1. Add **one nearby place** (a USB disk or a NAS share) and make the first backup by hand with **Back Up**.
2. Add **one place away from home**: Nextcloud over WebDAV, or a cloud folder, **encrypted**, with a passphrase you
   have written down.
3. Keep **Automatic backups**, *Copy when Bookx closes* and *Copy once a day* on. After the first copy they only add
   what is new, and your notes are protected within minutes.
4. Once in a while press **⋯ → Check Backup** on each place, and try **Restore into → Another folder** with *Preview*
   once to see that it works.

## Questions and answers

??? question "How do I make a backup right now?"
    Open the main menu, choose **Library Health** and pick **Backups** in the sidebar. On the **Back up now** row press
    **Back Up**. Bookx saves a snapshot of the catalogue and copies your books, covers, catalogue and notes to every
    place that is switched on. The first copy to a place takes everything; later ones only what is new. A progress bar
    shows the steps and **Cancel** stops it. Backups you start this way are never removed by the automatic clean-up.

??? question "How do I get automatic backups?"
    On the same Backups page, switch on **Automatic backups** for quick snapshots on this computer (when Bookx starts
    and closes, if something changed), and keep *Copy when Bookx closes* and *Copy once a day while Bookx is open* on
    for your other places. Each place has to have its first copy first (press **Back Up** once, or allow it in the place's
    settings); after that only new and changed files are copied. Your highlights, notes and bookmarks are also saved
    about every five minutes.

??? question "How do I back up to a USB drive or an SD card?"
    Plug the drive in, then on the Backups page press **Add a place** and choose the kind *Folder or external
    drive*. Press **Choose** and pick a folder on the drive. Mounted drives are recognised and labelled. When the
    drive is not plugged in later, the place shows *Unreachable* and Bookx simply tries again at the next backup.

??? question "How do I back up to a NAS or network share?"
    **Add a place**, kind *Network share (SMB, NFS, SFTP…)*. Type the address, for example `smb://nas/books` or
    `sftp://me@server/home/me`, and press **Connect** (or pick a share you already opened in Files under *Or pick a
    mounted share*). GNOME mounts it and may ask for a password. This needs GNOME's GVfs, which is normal on GNOME.

??? question "How do I back up to Nextcloud, encrypted?"
    **Add a place**, kind *WebDAV (Nextcloud, ownCloud…)*. Enter the address
    (`https://cloud.example.com/remote.php/dav/files/me/`), your user name, a password (an app password if you use
    two-factor sign-in) and the folder on the server. Leave **Encrypt the copies** on (it is on by default for WebDAV),
    then enter a **Backup passphrase** in *Copies to other places* on the Backups page. Press **Test** to check the
    connection. The files in the place are encrypted and only your passphrase can open them.

??? question "What if I forget the backup passphrase?"
    Then those encrypted backups cannot be opened by anyone, including you. There is no recovery. Write the passphrase
    down somewhere safe. Changing it later does not change copies that already exist: they still need the old one.

??? question "How do I use Google Drive or OneDrive?"
    Add the account in GNOME Settings → Online Accounts and switch on its *Files* option. In the Add a place dialog
    choose *Cloud drive (Google Drive, OneDrive…)*; your account is listed under *Connected accounts*. Bookx never
    sees your Google or Microsoft login, only the mounted folder. iCloud Drive cannot be used directly from Linux:
    choose a folder your iCloud sync client keeps on the computer, if you have one.

??? question "What does a backup contain? Are my books in it?"
    Yes. A backup holds your **book files**, their **covers**, and the **catalogue** with titles, authors, tags, series,
    collections, reading progress, bookmarks, highlights and notes. Not included: settings, plugins and saved
    passwords, the trash, copies on devices and caches. A place can be set to leave out the book files, the covers or
    files above a size you choose.

??? question "Is a backup incremental?"
    Yes. The first backup to a place copies everything; after that only new or changed files are copied. A renamed or
    moved book costs nothing, and the same book twice is stored once. A backup with nothing new copies nothing. Your
    highlights, notes and bookmarks are saved separately every few minutes, in very small records. Every file is read back and
    checked after it is written, and a bad one is removed and shown as *Failed*.

??? question "How do I restore a backup?"
    Open main menu → **Library Health** → **Backups**. **Restore from a backup** lists the quick catalogue snapshots on
    this computer; **Restore from another place** lists the backups of a place you pick. Press **Restore**, choose
    *This library* or *Another folder* and *Replace everything* or *Only the missing books*, press **Preview** to see
    what would change, then **Restore**. Before anything changes Bookx saves your current catalogue as a *Before a
    restore* snapshot, so you can go back. Encrypted backups use the saved passphrase or ask for it.

??? question "How do I restore on a new computer?"
    Install Bookx, open main menu → **Library Health** → **Backups**, press **Add a place** and choose the same drive,
    share or WebDAV folder (with the same passphrase if it was encrypted). Under **Restore from another place** pick
    that place and press **Restore** on the newest backup. Your books, covers and catalogue come back, with the notes
    up to a few minutes before the last backup. **Restore from a file** is for old `.db` or `.bookxbackup` files, which
    hold the catalogue only.

??? question "Why was nothing copied after I updated or added a place?"
    The first copy of a place is your whole library, so Bookx waits for you: the Backups page shows *Include your
    books in backups* with how big it is. Press **Back Up**, or switch on **Make the first copy automatically** in the
    place's settings. After the first copy the automatic copies run on their own.

??? question "Can I leave my books out of a backup?"
    Yes. In the place's settings switch off **Include book files** (and **Include covers**) to keep only the catalogue
    with your notes, or set **Largest file to copy** to leave out big files such as audiobooks. Files left out are listed.

??? question "How do I check that a backup works?"
    On the place, press **⋯ → Check Backup** (every file is there with the right size) or **Check Backup in Depth**
    (every file is read back and compared). To be sure, restore into **Another folder**: the preview shows what would come
    back and the restored folder opens as a library of its own.

??? question "How do I free space on a backup drive?"
    Press **⋯ → Remove Old Backups**. Bookx removes the backups beyond the number the place keeps (manual ones stay) and the
    files only they needed. Lower **Automatic backups to keep here** first for more.

??? question "A book disappeared. Can I get it back without a backup?"
    Look on the **Trash** page of Library Health first: deleted books wait there until you empty the trash, and the
    **History** list above it can undo recent changes such as edits and moves. Use a backup only to go back further.

??? question "Why does a place say Unreachable or Failed?"
    *Unreachable* means the drive is not plugged in, the share is not mounted or the server did not answer; the
    reason is shown and Bookx tries again next time. *Failed* means the place answered but the copy did not work:
    wrong password, no space or no permission. Other places are not affected. You can also switch a place off with its
    switch, or remove it (the copies already there stay).

??? question "Can two computers share one library through backups?"
    No. A backup is a copy at one moment; nothing flows back by itself and Bookx does not sync a library between
    computers. Reading position and highlights can be synced with KOReader or a device, see
    [Devices and sharing](devices-and-sharing.md#which-sync-do-i-need).
