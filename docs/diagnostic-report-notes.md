stub
| Case | Identified from the report alone | Area named | Why |
|---|---|---|---|
| `kosync-push-404-pdf-grimmory` | yes | Sync (KOSync client) | issue #49: the address shape, the matching method, the book's format and hash, and the HTTP status are all in the summary and the sync section |
| `boox-leaf2-expects-books-to-sync` | yes | Built-in server | issue #39: nothing failed; the report names the device as a Boox Leaf 2, shows the plugin and that it never completed a sync, and says what the server does and does not move |
| `package-built-for-another-release` | no | (no report possible) | the program does not start (glibc symbol error): the app cannot build a report; build-aux/triage-report.py recognises the pasted terminal output instead |
| `beta-notice-takes-focus` | partly | Display and interface | the report shows that a notice banner appeared and what the reader did right after; whether a key reached the page stays a question |
| `highlight-not-painted-after-relayout` | yes | Reader | the reader records how many highlights are stored and how many belong to the chapter shown, next to the relayout and the banner |
| `database-locked-two-writers` | yes | Library database | the open fails with class locked and the message names a second Bookx; the sync client and the file system are listed |
| `catalogue-schema-newer-than-supported` | yes | Library database | the error line, the supported schema and the copies from before an update are in the report |
| `webdav-backup-401` | yes | Backups | the backup job fails with class unauthorized and the place is told by kind, encryption and address shape |
| `webdav-backup-tls-error` | yes | Backups | class tls on the backup job, the scheme and the kind of host |
| `webdav-backup-timeout` | yes | Backups | class timeout with the duration of the job |
| `mtp-boox-not-mounted` | yes | Devices | the copy fails with class not_mounted, the device model, the backend and the sandbox device permissions |
| `kobo-database-missing` | yes | Devices | the device name is told as a model, the message says the Kobo has no library database |
| `kepub-conversion-fails` | yes | Conversion | the conversion step, its target, the input size class and the error class |
| `cbr-without-unrar` | yes | Conversion | the import fails for a cbr, and the helper line says neither unrar nor 7-Zip is there |
| `djvu-without-djvulibre` | yes | Conversion | the import fails for a djvu and the DjVuLibre line says it is missing |
| `pdf-without-text-layer-search` | partly | Search | search and read aloud on a scan fail with a generic error; the report shows the format and the failing step, but not that the PDF is a scan |
| `tts-voice-missing` | yes | Read aloud | open_engine fails with class missing, the engine and the helper lines |
| `tts-no-audio-player` | yes | Read aloud | the helper line says no audio player is installed and the sandbox sockets are listed |
| `tts-model-hash-mismatch` | yes | Read aloud | the download fails with class hash_mismatch and the job duration |
| `plugin-fails-to-load` | yes | Plugins and patches | the plugin id and version, the load failure and the warning in the log |
| `mcp-port-in-use` | yes | AI assistants (MCP) | the start fails with class port_in_use |
| `flatpak-without-network` | yes | Packaging | the sandbox section says the network is not shared, next to the failing sync call |
| `library-on-nfs` | yes | Library folder | the file system type and its kind are told, with the other folder facts |
| `library-on-fuse-rclone` | yes | Library folder | the FUSE file system and its kind |
| `disk-full-during-backup` | yes | Storage | the backup fails with class disk_full and the free space class of the library disk |
| `keyring-locked` | yes | Passwords and keyring | the settings line says passwords are in a locked store |
| `keyring-missing` | yes | Passwords and keyring | the settings line says passwords are not saved, with the unlock attempt |
| `hindi-layout-complaint` | partly | Reader | the script, the languages, the fonts and the renderer narrow it, but a wrong glyph or direction leaves no error to find |
| `huge-library-startup-stall` | yes | Display and interface | the start-up time, the stall events with the area and last operation, the library size and its disk |
| `dictionary-download-flaky` | yes | Metadata and lookup | the job fails with class timeout and says it was a dictionary download |
| `opds-server-401` | yes | OPDS and web import | the request fails with 401, the kind of host and the shape of the catalogue address |
| `calibre-import-unreadable-database` | yes | Import and add books | the import fails with its class and a message that says the file is not a database |
| `app-crashes-while-opening-a-pdf` | yes | Crash | the crash text with the message and the first frames, and the last operations before it (the book's format and size) |
| `server-port-busy` | yes | Built-in server | the start fails with class port_in_use and the port and sharing switches are listed |
| `news-feed-404` | yes | News and read later | the news job fails with the HTTP status of the feed |
| `device-copy-disk-full` | yes | Storage | the copy fails with class disk_full for the named device model and format |
| `full-text-index-empty` | yes | Search | the library section says how many books are indexed, with the disk and file system of the folder |
| `library-folder-read-only` | yes | Library folder | the folder section says it is read-only |
| `library-inside-syncthing` | yes | Library folder | the folder section names the sync client |
| `annotations-from-device-not-linked` | yes | Built-in server | the merge event counts the annotations that found no book, with the device model and the sync switches |
| `opds-lan-server-timeout` | yes | OPDS and web import | the request times out; the host is told as a private address and the scheme is plain http |
| `import-of-a-damaged-epub` | yes | Import and add books | the import fails with class corrupt, the format and the size class |
| `metadata-service-rate-limited` | yes | Metadata and lookup | the request fails with HTTP 429 and the kind of host |
| `settings-file-cannot-be-saved` | yes | Settings | the save fails with its class (read-only or full disk) and the folder facts |

40 of 44 cases are "yes" (90 percent).
