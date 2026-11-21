# Bookx Sync for KOReader

`bookx.koplugin` sends the highlights, notes and bookmarks of the open book to Bookx, and receives the
notes made in Bookx. Install: copy the whole folder `bookx.koplugin` into KOReader's `plugins` folder, replacing
any older copy (Bookx can do it for you from Preferences → Sync when the device is mounted), and restart KOReader.
The folder includes `bookxmd5.lua`; an update that leaves the old copy in place will not work. Then open the top
menu (tools tab → More tools) → Bookx Sync → Server and account.

## First connection from the e-reader

1. In Bookx, switch on Sync → KOReader sync (the address ends in `/bookx`, e.g. `http://192.168.1.20:8765/bookx`).
2. In KOReader: top menu → tools tab → More tools → Bookx Sync → **Server and account**.
3. Fill in the server address, a user name and a password, then tap **Register account** if you have no account
   yet (it creates the account, saves it and tests it), or **Save** if the account already exists.
4. **Save** answers "Saved. Password is kept as a hash; the field will be empty next time." with the server and
   user. Only the MD5 hash of the password is stored (in `settings/bookx_sync.lua`), never the password, so the
   password field is empty when you reopen the dialog; its hint reads "Saved — leave empty to keep". Type a new
   password only to change it.
5. **Test connection** (a button in the dialog and a menu item) calls `<server>/users/auth` and says one of:
   "Connected as <user>", "Wrong user name or password", "Bookx sync is not enabled at that address (404)",
   "Cannot reach <host:port> (connection refused / timed out / unknown host)" and, with a newer or older Bookx, a version message. The first menu line shows the state: Not configured, Not tested yet,
   Last test OK/failed, Last sync.
6. Register errors: "name taken" (choose another name, or use **Save** with the password of the existing
   account), "registration is closed" (the server owner closed it in Bookx).

An address without a path gets `/bookx` added. Only `/bookx` is used.

Licence: MIT (see `LICENSE`), the plugin is original code; nothing is copied from KOReader or other plugins.
It uses KOReader's own `socket.http` and `ltn12` at run time; the password hash is computed by the bundled
`bookxmd5.lua`, so a missing KOReader `ffi/sha2` module cannot crash the plugin.

Version 0.3.1 (every request names the device and versions for Bookx's *Connected devices*; automatic sync after a highlight, on open and when Wi-Fi comes back, and a visible *Waiting for Wi-Fi* instead of a silent skip; the summary says what was sent and received; About / Diagnostics lists the books Bookx could not match). **Bookx Sync → About / Diagnostics** shows the version, server, last test/sync/error (with the Lua traceback) and
**Copy diagnostics** writes `settings/bookx_diagnostics.txt`. Errors are logged to KOReader's `crash.log` with the full traceback
(`Bookx Sync error in …`); the screen gets one short message. `build-aux/package-plugin.sh` builds `dist/bookx.koplugin.zip`.

Tests: `koreader-plugin/tests/run.sh` (needs `lua` or `luajit`; it runs the sync logic and the real `main.lua` menu
under a fake KOReader, see `docs/koreader-plugin-testing.md` for how to add a test). Setting `BOOKX_KO_FRONTEND` to a KOReader installation additionally exercises
the real settings, logger, gettext and `ffi/sha2` modules. KOReader-facing errors are contained with `pcall` and
reported without showing a stack trace on the device.
