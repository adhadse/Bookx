# Backup and restore

A **backup** in Bookx is a snapshot of your *catalogue*, not of your book files. Menu → **Library Health** shows what
is included, where backups go and lets you restore them.

!!! important "What a backup contains"
    | Included | Not included |
    |---|---|
    | Catalogue metadata: titles, authors, tags, series, collections and identifiers | Book files and their `cover.jpg` files |
    | Reading progress, bookmarks, highlights and notes | Configuration, plugins and saved passwords |
    | The full-text search (FTS) index | `.bookx/` state, the trash, copies on devices and caches |

    Your book files stay in the library folder. Back that folder up with whatever you already use for your documents;
    Bookx backups cannot bring the books back. Restore a catalogue backup and point the library folder at your book
    files to put the two together again.

### Not incremental

A Bookx backup is a **whole-file snapshot** of the catalogue. Bookx does not store incremental changes or deduplicate
parts of the database. It skips a place when the snapshot has not changed and skips a copy a place already has; when
retention removes older snapshots it deletes whole files. Every copy is read back and checked with SHA-256 after it is
written.

!!! note "Backups are one-way snapshots, not sync"
    A backup is a copy at one moment. Nothing flows back by itself, and two computers cannot use one library through
    backups. Bookx does not sync a library between computers. See [Which sync do I need?](devices-and-sharing.md#which-sync-do-i-need)
    for what Bookx can sync (reading position with KOReader, a book list with a device).

## Where backups go

*Library Health → Where backups go* lists every place with its **path or address**, how many **backups** it holds,
their **size**, the **time of the last one**, the free space when the place can tell, and a **status**:

| Status | Meaning |
|---|---|
| OK | The last copy worked (and was checked, see below). |
| Unreachable | The drive is not plugged in, the share is not mounted, the server did not answer. The reason is shown. Bookx tries again next time. |
| Failed | The place answered but the copy did not work: wrong password, full, no permission. The reason is shown. |
| Not used yet / Switched off | A new place, or one you switched off with its switch. |

**On this computer** is always there: the library folder's `.bookx-backups` folder. You can add any number of other
places; every backup goes to all of them that are switched on. A place that is down does not stop the others.

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
its own **number of automatic backups to keep**. Manual backups (*Back up now*) are never removed.

## Making backups

<figure markdown="1">
![The Backups tab of Library Health](assets/library-health-backups-light.png#only-light)
![The Backups tab of Library Health](assets/library-health-backups-dark.png#only-dark)
<figcaption markdown="1">*Backups*: automatic snapshots, how many to keep, *Back Up* now and the places backups are copied to.</figcaption>
</figure>


- **Automatically on this computer**: a snapshot when Bookx starts and closes, if something changed.
- **To other places**: *Copy when Bookx closes* and *Copy once a day while Bookx is open* (both on by default for the
  places you add). Only a snapshot a place does not have yet is copied. When Bookx closes it waits at most 20 seconds
  for a place that does not answer.
- **Back up now**: takes a snapshot and copies it to every switched-on place, with a progress bar and **Cancel**. A
  cancelled copy is removed from the place, so no half-written file is left.

### Checked after writing

After every copy Bookx reads the file back from the place and compares its SHA-256 with what it wrote. If they differ the
bad copy is deleted and the place shows *Failed*, so a full disk or a flaky share cannot silently give you an unusable backup.

## Encrypting copies

Switch on **Encrypt the copies** for a place (it is on by default for WebDAV) and set the **Backup passphrase** under
*Copies to other places*. The copies are then `.bookxbackup` files:

- the catalogue is encrypted with **ChaCha20-Poly1305**; the key is made from your passphrase with **Argon2id**, a
  deliberately slow function, so a stolen file cannot be guessed at quickly;
- the file says what it is (Bookx version, date, number of books) in a readable header, but nothing else; a changed
  header, a changed byte or a cut-off file is detected when restoring;
- the passphrase is kept in the system keyring, not in the settings file ([Passwords](#passwords)).

!!! warning "Do not lose the passphrase"
    There is no recovery. Without the passphrase an encrypted backup cannot be opened by anyone, including you. Write
    it down somewhere safe. Changing the passphrase does not change the copies already made; they still need the old one.

Plain copies are ordinary SQLite files (`.db`), which you can open with any SQLite tool. Anyone who can read the place
can read them: Bookx shows a note under every off-site place that is not encrypted.

## Restoring

<figure markdown="1">
![The Trash and History tab of Library Health](assets/library-health-trash-light.png#only-light)
![The Trash and History tab of Library Health](assets/library-health-trash-dark.png#only-dark)
<figcaption markdown="1">*Trash & History*: removed books can be restored, and the log of changes can be undone.</figcaption>
</figure>


*Library Health* has two lists:

- **Restore from a backup** shows the snapshots on this computer.
- **Restore from another place**: pick a place; its backups are listed (date, number of books, kind, size). Press
  **Restore** on one. Encrypted ones use the saved passphrase, or ask for it. *Restore from a file* takes a `.db` or
  `.bookxbackup` file you copied yourself, for example on a new computer.

Before anything changes, Bookx checks the backup for damage and saves the **current** catalogue as a *Before a
restore* snapshot, so a restore can be undone by restoring that one. Restoring on a new computer: install Bookx, open
*Library Health*, add the place again (or choose the file) and restore. Then point the library folder at the folder
that holds your book files; those files are not in the backup.

## Command line

```bash
bookx-cli library backup --to /run/media/me/BACKUP/bookx           # a folder or a mounted share
bookx-cli library backup --to https://cloud.example.com/remote.php/dav/files/me/ --user me --encrypt
bookx-cli library snapshots --from /run/media/me/BACKUP/bookx      # what is there
bookx-cli library restore latest --from /run/media/me/BACKUP/bookx
bookx-cli library restore ~/Downloads/bookx-20261112T101500Z-n120-manual.bookxbackup
```

Secrets are never taken from the command line: the WebDAV password comes from `BOOKX_WEBDAV_PASSWORD` and the
passphrase from `BOOKX_BACKUP_PASSPHRASE`. `smb://` addresses must be mounted first (`gio mount smb://nas/books`) and the
folder under `/run/user/<id>/gvfs/` given. `--keep N` sets how many automatic backups the place keeps.

## Passwords

Bookx keeps passwords (sharing, e-mail, reading position sync, WebDAV for devices and backups, the backup passphrase)
in the **system keyring** (GNOME Keyring, KWallet, KeePassXC; in the Flatpak through the secrets portal), not in the
settings file. Passwords saved by older versions are moved there the first time Bookx starts, and removed from the file.

If no keyring is available, Bookx keeps working and says so next to each password: it is then kept in the settings
file, unencrypted. Install GNOME Keyring or KeePassXC and the next save moves them.

## Limits

- The backup is read into memory to encrypt and check it. Catalogues of hundreds of megabytes (with full-text indexes of
  very large libraries) need that much free memory, and WebDAV downloads are limited to 2 GB.
- The progress bar moves between steps (reading, encrypting, copying, checking), not byte by byte.
- A WebDAV copy cannot be cancelled in the middle of an upload; the cancel is taken after it and the file is removed.
- Bookx has not been tried against Google Drive, OneDrive or a real Nextcloud with real accounts; these routes use
  GNOME’s mounts and standard WebDAV, which are tested with local stand-ins.
