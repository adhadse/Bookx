# KOReader highlights and notes (Wi-Fi)

Bookx can receive the highlights, notes and bookmarks you make in **KOReader**, and send the highlights you make in
Bookx back, over your Wi-Fi. This works only with KOReader (on any device that runs it: Kobo, Kindle, Android, PocketBook
and others). Devices with their own firmware and no KOReader use the USB cable instead: see
[Devices and sharing](devices-and-sharing.md).

It is part of **Bookx sync (extended KOSync)**: Bookx’s built-in server plus a small KOReader plugin that Bookx can
copy to the device. Open the main menu (the ☰ button in the library's header bar) and choose **Sync** for the overview,
or choose **Preferences** and its **Sync** page for the setup; see [Bookx sync](progress-sync.md).

!!! warning "Hardware tested so far"
    The KOReader plugin has been run against a simulated KOReader and replayed with the real notes files of one Kobo
    (KOReader 2026.07.2). A first sync from a real device with the current plugin (0.3.1) is not confirmed yet, and
    only that one Kobo has been looked at. Live updates in the open reader (a highlight or a position from the device)
    are tested with a simulated device. Keep your highlights on the device as they are: Bookx never deletes or lowers
    anything there.

## Set it up

Open the main menu → **Preferences** → **Sync** and follow the **Setup checklist** at the top of the page:

1. **Turn on the Bookx sync server.** The address for the e-reader appears under the switch, ending in `/bookx`.
   Bookx remembers the switch: if the server was on when you closed Bookx, it starts again by itself the next time Bookx
   opens, and if you switched it off it stays off. If it cannot start (for example the port is taken) the page says why
   and Bookx remembers that you want it on.
2. **Create your sync account** ([which user name and password?](#account)): type a user name and a password of your
   choice and press **Register**. The step then shows **Logged in as** your name; **Log in** checks an existing account
   again.
3. **Install the Bookx plugin on KOReader.** Connect the e-reader with the USB cable and press **Copy to the device**.
   Bookx finds KOReader's `plugins` folder (`.adds/koreader/plugins` on a Kobo, `koreader/plugins` elsewhere) and puts
   the plugin `bookx.koplugin` there. Without a cable, press **Save to a folder** and copy the folder `bookx.koplugin`
   into KOReader's `plugins` folder yourself. Restart KOReader.
4. **Test on the device.** In KOReader open a book → top menu → **More tools** → **Bookx Sync** → **Server and account**:
   type the address, the user name and the password; then **Sync now**, or switch on **Sync automatically** to sync
   when the book is closed and when the device goes to sleep.

**Sync highlights, notes and bookmarks** (under *What Bookx sync does*) is **on by default**; it only matters while the
Bookx sync server is on, and you can switch it off there. The setup checklist is a stepper (a numbered step with a
status and a check mark when done); the main menu → **Sync** window shows the same four steps in a compact stepper and has a
**Sync now** button once they are done.

## Which user name and password? { #account }

The user name and password are your **Bookx sync account**. There is no pre-existing KOReader account and they are
**not** the OPDS server password and **not** a Bookx login.

1. **Create it in Bookx**: main menu → **Preferences** → **Sync**, step *2. Create your sync account*, then **Register**. Or in KOReader:
   **Tools → Progress sync → Custom sync server**, the same address Bookx shows, `http://<host-ip>:<port>/bookx`, then **Register**.
2. **Then, in Bookx Sync → Server and account** on the e-reader, enter the same server address, that same user name and
   that same password. The plugin keeps only a one-way hash of the password; afterwards you can leave the password field
   empty to keep the saved one. **Device name** is only a label shown in Bookx.
3. Highlight sync and reading-progress sync use the **same account over the same `/bookx` connection**.

Stock Kobo and Kindle firmware has no wireless sync with Bookx; those readers sync over the USB cable only.

!!! warning "What is stored on the e-reader"
    The plugin never stores your password. It stores the **MD5 hash** of it (the *key*) in KOReader's settings
    (`settings/bookx_sync.lua`), because the KOReader sync protocol sends that hash with every request and KOReader has no
    keyring. MD5 is fast to guess, and the key works as a login on its own, so anyone who can read the e-reader's files
    can sync as you. Use a sync password **that you use nowhere else**, and treat the e-reader's storage as unprotected.
    On the computer side, Bookx does not keep that key either: the sync server stores only a salted Argon2id hash of it
    (older files are converted the first time the server starts), and the sync password in Bookx lives in the
    [keyring or the encrypted password store](backup-and-restore.md#passwords). To revoke a lost e-reader, change the
    sync account password in Bookx and enter the new one on your devices.

## What happens

- Highlights, the notes you wrote on them and bookmarks go from KOReader to Bookx. A highlight you delete in KOReader is
  removed in Bookx too, but only if it came from KOReader in the first place and you have not changed its note in Bookx.
  **You can get it back:** a message says “Kobo Libra removed 3 highlights from *Dune*” with **Undo**, and the
  [History](annotations.md#history) lists every removal so you can restore it later. A restored highlight is not deleted by
  the next sync; it is sent to the device as a new highlight.
- Highlights are **placed by their text**, not by KOReader’s internal position. Bookx looks the quote up in its own copy of the
  book. A quote it cannot find is kept as a note *without a place*: it appears in **Notes & Highlights** as “Could not be located
  in the book” (you can try to locate it again), and is never painted on the wrong page.
- A quote you already have in Bookx is not added twice; its note is filled in only if you had none.
- **Colours and styles come along.** Every KOReader highlight colour (red, orange, yellow, green, olive, cyan, blue,
  purple, gray) and style (highlight, underline, strikethrough, invert) arrives as you set it, and a colour you change in
  KOReader later reaches Bookx at the next sync. A colour you chose yourself in Bookx is not reset by the device unless you
  change it on the device again. A colour or style Bookx does not know (a newer KOReader may add some) is kept as written and
  handed back unchanged. Highlights Bookx already had in the default yellow are given the device’s colour when the device sends
  them; see [Colours and styles from your e-reader](annotations.md#colours-and-styles-from-your-e-reader).
- In **Notes & Highlights** a highlight that came from the device shows “KOReader (device name)”.
- If the same note was changed in both places, Bookx keeps yours and counts it in the sync line of the window.
- Highlights you make in Bookx are sent for the books the device has synced at least once (not for other books), with their colour
  and style in KOReader’s words (pink goes as red, a custom colour as the nearest KOReader colour). On the device,
  **Bookx Sync → Notes from Bookx for this book** lists them. KOReader does not paint them on the page.
- **Bookmarks come along as bookmarks.** A page bookmark you make in KOReader becomes a Bookx bookmark, listed under
  **Bookmarks** in *Notes & Highlights* and in the reader’s **Bookmarks** tab, showing “KOReader (device name)”. A bookmark has no
  text to look up, so Bookx works out the place from what the device sent: KOReader’s position in the book when it can read it
  (the page you bookmarked), otherwise the chapter, the percentage or the page number. The last three are a guess; such a
  bookmark says *approximate place* and may be a few pages off. Words you wrote on the bookmark become its label.
- A bookmark you already have in Bookx at that place is not added twice and is never changed. A bookmark you delete in Bookx is
  **not brought back** by the next sync, and one you delete on the device is **not deleted** in Bookx.
- The sync line of the window counts the bookmarks it added and the ones that were already in your library.
- Syncing the same thing twice never duplicates anything. If the connection fails, nothing is lost: the next sync repeats it.

## Seeing what synced { #activity }

Open the main menu → **Sync** (or **Preferences → Sync**). Under the setup you find:

- **Connected devices**: one row per e-reader that has talked to Bookx, with its name, the KOReader and plugin version, when it
  last synced, and a status: **Connected now** (it was in touch in the last two minutes), **Last seen 5 minutes ago** or
  **Never connected**. A device with an older plugin, or one that only uses KOReader's Progress sync, shows as *Unknown device
  (KOReader)* or by the name KOReader gives it.
- **Recent activity**: the newest syncs, most recent first (the last 200 are kept; **Show All** lists them). Each row names the book, the
  device, the time and what happened: *Progress 12% → 34%*, *3 highlights added or updated, 2 with notes, 1 bookmark*,
  *1 highlight could not be placed in the book*, *Book not found in your library*, or *Server rejected it* with the reason.
  The line above the list counts the day (“Today: 12 highlights, 3 bookmarks, progress for 2 books”). Press a row to open that book in
  **Notes & Highlights**.
- **Books not found in your library**: books the device sent highlights for that Bookx has no matching book for. Nothing is
  imported for them (and nothing is lost: add the book to your library, press **Sync now**, and the waiting highlights are imported).
  The row shows the title the e-reader gave and a short code of the book.

While Bookx is open, a message appears when a sync arrives (“Synced 3 highlights from Kobo Libra for Building Microservices”) with
a **Show** button that opens the book in **Notes & Highlights**; the counts there and in the sidebar update without reopening
anything. The details of a book say when and from which device it last synced.

On the e-reader, **Bookx Sync → About / Diagnostics** shows the same from its side: the last sync, how much was sent and received,
whether a sync is waiting for Wi-Fi, and the books Bookx said it has no match for. After **Sync now** the screen says what was sent
(“Sent 3 highlights, received 1 note from Bookx”) and warns when Bookx does not have the book.

## When does “Sync automatically” sync? { #auto-sync }

With **Bookx Sync → Sync automatically** on, the plugin syncs about **30 seconds after the last change** to a highlight, note or
bookmark, when you **close the book**, when the device **goes to sleep**, and when you **open a book** with Wi-Fi on. It never switches
Wi-Fi on by itself (that would drain the battery). If Wi-Fi is off at that moment, the plugin remembers that something is waiting
(the Bookx Sync menu says *Waiting for Wi-Fi*) and sends it the next time a book is open with Wi-Fi on. To send now, turn Wi-Fi on and
choose **Sync now**.

## Live updates { #live }

While Bookx is open, what the e-reader sends shows up **at once**, without closing and reopening anything. If the book is open in the
reader, new highlights are drawn on the page, a changed colour or style changes on the page, a highlight you removed on the e-reader
disappears, and the **Bookmarks** and **Notes** tabs of the sidebar, the page bookmark button and the [book map](reading.md#book-map)
follow. The **Notes & Highlights** view, its counts and the details of the book in the library follow too, and the page stays where
it is: your selection, the open popover and a note you are typing are left alone.

- **A note you are editing** is never replaced behind your back. If the same note changed on the e-reader, a small message at the top
    of the page says *This note changed on your e-reader* with **Use Theirs** and **Keep Mine**.
- **Reading position.** When the e-reader reports that it is further along than you, a message offers *Your Kobo Libra is at 62%
    (page 143)* with **Continue There** and **Dismiss**. The page never moves by itself. The message goes away after 15 seconds or when you
    turn a page. Switch it off with **Preferences → Sync → Offer to jump when a device is ahead**. Your own reading progress is never
    lowered.
- The live part is on the Bookx side and takes about a second. **How soon the e-reader sends is up to the plugin**: it syncs about 30
    seconds after you make a highlight (see [below](#auto-sync)), so “real time” starts when the plugin sends, not when you lift your finger.

## Limits

- Bookx knows a book of the device by KOReader’s book fingerprint (the same one the progress sync uses). If the copy on the
  device is not the same file as in your library (converted, for example), Bookx falls back to the title and author; books it
  cannot match are reported as “not in your library”.
- A highlight of fewer than 8 characters (a single short word such as “REST”) is kept as a note without a place: Bookx never guesses
  where such a word belongs.
- KOReader writes times without a time zone; Bookx shows them as written, so a highlight may show a few hours off if the computer and the
  device use different zones.
- Needs a recent KOReader (one that has the annotations list). The plugin says so if yours does not.
- The server stores at most 5000 annotations per book and accepts 120 requests a minute per account. Texts and notes are
  limited to 20 000 characters.
- Use plain HTTP on a network you trust: KOReader does not trust Bookx’s self-signed HTTPS certificate.
- Other e-readers, and the Kobo or Kindle firmware itself, are not synced over Wi-Fi. Use the USB cable and the
  [Device window](devices-and-sharing.md).

## Your data

Annotations received from devices are kept in Bookx’s data folder (`koreader-annotations.json`) for the account that sent
them; the library receives them as normal highlights. Switching the option off stops the server routes; nothing already in your
library is removed.

## If it does not work { #troubleshooting }

The plugin always says what went wrong in one line, and keeps the details:

| Message on the e-reader | What to do |
|---|---|
| *cannot reach 192.168.1.5:8080 (connection refused)* | Bookx is not running, the sync server is off, or the port is wrong. Check the address in **Server and account** against the one Bookx shows. |
| *cannot reach … (timed out)* / *(unknown host)* | The e-reader is not on the same network as the computer, or a firewall blocks the port. |
| *wrong user name or password* | Type the password again in **Server and account** and press **Save**. |
| *the Bookx server has no KOReader sync here* (404) | The address must end in `/bookx`, and the sync switch must be on in Bookx. |
| *This Bookx is too old for the plugin* / *Bookx needs a newer Bookx Sync plugin* | Update Bookx, or reinstall the plugin from Bookx (**Preferences → Sync → Copy to the device**). |
| *Bookx has no matching book in your library* (after Sync now) | The book on the e-reader is not in your Bookx library, or it is a different file with a different title. Add the book to Bookx and sync again; the highlights are kept on the server meanwhile. |
| *Waiting for Wi-Fi* in the Bookx Sync menu | Automatic sync does not turn Wi-Fi on. Turn it on and open a book, or choose **Sync now**. |
| *Bookx sync: unexpected error: …* | A fault in the plugin. Open **Bookx Sync → About / Diagnostics**. |

**About / Diagnostics** (top menu → More tools → Bookx Sync) shows the plugin version, the server, the last test, the last sync
and the last error with its details. **Copy diagnostics** saves the same text as `bookx_diagnostics.txt` in KOReader's `settings`
folder (on a Kobo: `.adds/koreader/settings`), without your password; send that file if you ask for help.

KOReader's own log is the file `crash.log` in KOReader's folder (on a Kobo: `.adds/koreader/crash.log`). Lines from the plugin
start with `Bookx Sync`, and an error is followed by the full trace. The log starts afresh every time KOReader starts, so look at
it before you restart KOReader.

After updating Bookx, install the plugin again and restart KOReader; **About / Diagnostics** shows which version is running.

## Using a reverse proxy

If Bookx sits behind a proxy, the address in the plugin is the public one, for example `https://bookx.lan/bookx`; the
proxy must not remove `/bookx`. See [Put Bookx behind a reverse proxy](reverse-proxy.md). A proxy with a certificate
that KOReader trusts also solves the self-signed HTTPS limit above.

## Questions and answers

??? question "How do I start the Bookx sync server?"
    Open the main menu → **Preferences** → **Sync**. Step 1 of the *Setup checklist* is **Turn on the Bookx sync
    server**. Switch it on: Bookx starts the server, opens it to your home network so the e-reader can reach it and, if
    you have none yet, makes a library password (shown under **Advanced**). The address for the e-reader, ending in
    `/bookx`, appears under the switch with a copy button. The main menu → **Sync** window has a **Set up Bookx sync**
    button that takes you through the same steps.

??? question "Why did the server switch itself off? Does it stay on after I restart Bookx?"
    It stays as you left it. Bookx remembers whether the server was on, and starts it again when Bookx opens. If you
    switch it off, it stays off. Only if it cannot start (for example the port is used by another program) does the
    page show the reason; Bookx still remembers that you want it on. Note that the server runs only while Bookx is
    running; for an always-on server use `bookx-cli serve`, see [Command line](command-line.md#serve).

??? question "How do I create the sync account? Which user name and password?"
    In Preferences → Sync, step 2 **Create your sync account**: type any user name and password and press
    **Register**. There is no existing KOReader account; you make it here. Afterwards the step shows **Logged in as**
    and your name. It is not the library password (user `bookx`) and not a login to anything online. **Log in** checks
    that an existing account still works.

??? question "How do I install the plugin on KOReader?"
    Step 3 of the checklist. Connect the e-reader with USB and press **Copy to the device**: Bookx finds KOReader's
    `plugins` folder and puts `bookx.koplugin` there. Without a cable press **Save to a folder** and copy that folder
    into KOReader's `plugins` folder yourself. Then restart KOReader.

??? question "How do I test that it works?"
    Step 4. On the e-reader open a book, then the top menu → **More tools** → **Bookx Sync** → **Server and account**: type
    the address, the user name and the password. Then choose **Sync now**. In Bookx, highlights from the device show up
    in **Notes & Highlights** marked “KOReader (device name)”. In the main menu → **Sync** window the status changes
    to *Bookx sync is connected* once a device has synced.

??? question "How do I sync automatically?"
    On the device: **Bookx Sync** → tick **Sync automatically**. It then syncs about 30 seconds after a
    highlight, when you close a book, when the device goes to sleep and when you open a book, as long as Wi-Fi is on (the plugin never
    turns it on; a sync that finds Wi-Fi off waits, see [When does “Sync automatically” sync?](#auto-sync)). Reading position itself is kept in step by KOReader's own Progress
    sync when you point it at the same address.

??? question "Do highlights from my e-reader appear while I read?"
    Yes. With the book open in Bookx, a highlight, note, bookmark or colour change that the e-reader sends is drawn on the page and
    listed in the sidebar about a second after it arrives, and the Notes & Highlights view updates as well. The e-reader sends it about
    30 seconds after you make it (or at once with **Sync now**), so that is the delay you see. A note you are typing in Bookx is not
    replaced: you are asked. See [Live updates](#live).

??? question "Why don't my highlights show up in Bookx?"
    Check in this order. 1. **Is the plugin the current one?** On the e-reader, **Bookx Sync → About / Diagnostics** must show a
    version of 0.3.1 or newer and a last sync; an error there is the answer (send the *Copy diagnostics* file). 2. **Did it sync?**
    The Bookx **Sync** window lists every sync under **Recent activity**. If your device is not under **Connected devices**, it never
    reached Bookx: check the address (ending in `/bookx`), the port in your firewall and that Wi-Fi was on. The plugin does not
    switch Wi-Fi on by itself, so with *Sync automatically* a highlight made with Wi-Fi off waits (*Waiting for Wi-Fi*) until you
    open a book with Wi-Fi on or choose **Sync now**. 3. **Does Bookx have the book?** Bookx finds the book by KOReader's fingerprint
    of the file, then by title and author. A copy that was converted or re-saved has a different fingerprint, and a book that is not
    in your library at all is listed under **Books not found in your library**; add it, then press **Sync now**. 4. **Could the quote be
    found?** A highlight whose text is not in Bookx's copy of the book, or is shorter than 8 characters, shows in *Recent activity*
    as “could not be placed” and in **Notes & Highlights** as “Could not be located in the book”. 5. **The log:** KOReader's
    `crash.log` (on a Kobo `.adds/koreader/crash.log`), lines starting with `Bookx Sync`.

??? question "Do I need the plugin for reading progress?"
    No. KOReader's built-in **Tools → Progress sync → Custom sync server** works against the same `/bookx` address and
    account. The plugin adds highlights, notes and bookmarks.

??? question "Where do I see my bookmarks?"
    In Bookx, click **Notes & Highlights** and choose **Bookmarks** at the top of the list (or **All** to see them with the
    highlights). The ones from KOReader say “KOReader (device name)”. In the reader they are in the sidebar under
    **Bookmarks**. Click one to open the book there. On the device they are in KOReader’s own bookmark list.

??? question "How do I stop highlights from being synced?"
    In Preferences → Sync, switch off **Sync highlights, notes and bookmarks** in the *What Bookx sync does* group. The
    server then serves no annotations. Nothing already in your library is removed.

??? question "Why does a highlight say “Could not be located in the book”?"
    Bookx places a highlight by searching its text in your copy of the book. If the quote is not found, or it is shorter
    than 8 characters, it is kept as a note without a place instead of being painted on a wrong page. Open it in
    **Notes & Highlights** to try again.

??? question "Why are some of my books “not in your library”?"
    Bookx recognises a book by KOReader's file fingerprint, and falls back to title and author. A copy on the device that
    was converted, so it is not the same file, may not match.

??? question "Does it work with the stock Kobo or Kindle software?"
    No. Wi-Fi sync needs KOReader on the e-reader. Stock firmware syncs over the USB cable only: see
    [Devices and sharing](devices-and-sharing.md).

??? question "Can I use it from outside my home, or over HTTPS?"
    Yes, through a [reverse proxy](reverse-proxy.md) that keeps the `/bookx` path and has a certificate KOReader
    trusts. Bookx's own self-signed HTTPS certificate is not trusted by KOReader, so on your home network use plain HTTP.

??? question "The device cannot connect. What should I check?"
    Both must be on the same network and the port must be allowed in your firewall. Check that the server is on (step 1),
    that the address ends in `/bookx`, and that user name and password match those of step 2. Bookx Preferences → Sync
    shows a hint about the firewall under the address.

??? question "Where are the received highlights stored?"
    In Bookx's data folder (`koreader-annotations.json`), and as normal highlights in your library. Switching the option
    off stops the server routes only.
