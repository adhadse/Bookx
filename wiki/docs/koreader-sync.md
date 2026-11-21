# KOReader highlights and notes (Wi-Fi)

Bookx can receive the highlights, notes and bookmarks you make in **KOReader**, and send the highlights you make in
Bookx back, over your Wi-Fi. This works only with KOReader (on any device that runs it: Kobo, Kindle, Android, PocketBook
and others). Devices with their own firmware and no KOReader use the USB cable instead: see
[Devices and sharing](devices-and-sharing.md).

It is part of **Bookx sync (extended KOSync)**: Bookx’s built-in server plus a small KOReader plugin that Bookx can
copy to the device. Open the main menu (the ☰ button in the library's header bar) and choose **Sync** for the overview,
or choose **Preferences** and its **Sync** page for the setup; see [Bookx sync](progress-sync.md).

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
**not** the Share Library password and **not** a Bookx login.

1. **Create it in Bookx**: main menu → **Preferences** → **Sync**, step *2. Create your sync account*, then **Register**. Or in KOReader:
   **Tools → Progress sync → Custom sync server**, the same address Bookx shows, `http://<host-ip>:<port>/bookx`, then **Register**.
2. **Then, in Bookx Sync → Server and account** on the e-reader, enter the same server address, that same user name and
   that same password. The plugin keeps only a one-way hash of the password; afterwards you can leave the password field
   empty to keep the saved one. **Device name** is only a label shown in Bookx.
3. Highlight sync and reading-progress sync use the **same account over the same `/bookx` connection**.

Stock Kobo and Kindle firmware has no wireless sync with Bookx; those readers sync over the USB cable only.

## What happens

- Highlights, the notes you wrote on them and bookmarks go from KOReader to Bookx. A highlight you delete in KOReader is
  removed in Bookx too, but only if it came from KOReader in the first place and you have not changed its note in Bookx.
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
    On the device: **Bookx Sync** → **Server and account** → switch on **Sync automatically**. It then syncs when you close
    a book and when the device goes to sleep. Reading position itself is kept in step by KOReader's own Progress sync
    when you point it at the same address.

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
