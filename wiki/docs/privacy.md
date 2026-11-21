# Privacy

Bookx has **no telemetry, no analytics, no account and no update check**. It never contacts anyone on its own: every
connection below happens because you used a feature that needs it. Your library, your notes and your reading history
stay in files on your computer (see [Library files](library.md#library-files-and-layout)).

## Offline mode

<figure markdown="1">
![The Offline mode switch](assets/privacy-offline-light.png#only-light)
![The Offline mode switch](assets/privacy-offline-dark.png#only-dark)
<figcaption markdown="1">*Offline mode* refuses every look-up and download until you switch it off.</figcaption>
</figure>

<figure markdown="1">
![Preferences, Privacy](assets/preferences-privacy-light.png#only-light)
![Preferences, Privacy](assets/preferences-privacy-dark.png#only-dark)
<figcaption markdown="1">Preferences → Privacy: the offline switch and what Bookx can connect to.</figcaption>
</figure>


The **Offline mode** switch on the **Privacy** page of Bookx Preferences (main menu → Preferences) refuses every online look-up and download: metadata and covers, Wikipedia,
translation, online catalogues, web articles and news, link checks, dictionary and voice downloads. Features then say
that offline mode is on instead of failing slowly. Books, notes, search, conversion, editing, reading aloud with a voice
you already have and your backups keep working.

Servers you set up yourself are **not** stopped by the switch, because you chose their address: the voice server, the
mail server, WebDAV folders, the sync server and Readwise. The library server (Sync) is not a connection from your computer:
other devices connect to it, only while it is on and only with the password. The environment variable
`BOOKX_OFFLINE=1` has the same effect for one session.

## What Bookx can connect to

| Feature | Where | What leaves your computer | Offline mode |
|---|---|---|---|
| Fetch Metadata and covers | openlibrary.org, covers.openlibrary.org, googleapis.com (Google Books), query.wikidata.org | title, author, ISBN of the books you look up | stopped |
| Wikipedia look-up in the reader | `<language>`.wikipedia.org, upload.wikimedia.org (pictures) | the selected word or phrase | stopped |
| Translate in the reader | translate.googleapis.com | the selected text | stopped |
| Online catalogues | gutenberg.org, standardebooks.org and the catalogues you add | your search words; books you download | stopped |
| Add from the Web and news subscriptions | the addresses you give | a plain request for the page or feed | stopped |
| Check Links in the editor | the web addresses found in the book | a plain request per link | stopped |
| Dictionary downloads | download.wikdict.com, github.com | the file you ask for | stopped |
| Voice and model downloads | huggingface.co, github.com | the file you ask for | stopped |
| Read aloud with a voice server | the server address you enter | the text being read | not stopped |
| Send by e-mail | the mail server you enter | the book and the message | not stopped |
| Cloud folders (WebDAV) for devices and backups | the WebDAV address you enter | book files; backups of your books, covers, catalogue and notes (encrypted if you chose to) | not stopped |
| Bookx sync (reading position, highlights, notes) | the sync server you enter, or your own Bookx | a book fingerprint and your position | not stopped |
| Sync: library server (OPDS, KOReader) | nothing: other devices connect to this computer | the books you share | not stopped |
| Readwise upload | readwise.io | your highlights, when you press Upload | not stopped |

The same list is on the Privacy page of Preferences. Translate sends the selected text to Google's translation service; if that
is not acceptable for a book, leave the feature alone or turn offline mode on. The look-ups never send your library,
your reading history or any identifier of your computer.

!!! note "Limits"
    - Plugins can only contact the hosts named in their manifest, and their requests go through the same HTTP
      client, so offline mode stops them too (see [Plugins and patches](plugins-and-patches.md)).
    - Passwords for mail, WebDAV and similar are kept in the system keyring, not in the settings file.
