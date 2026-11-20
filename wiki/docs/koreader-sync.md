# KOReader highlights and notes (Wi-Fi)

Bookx can receive the highlights, notes and bookmarks you make in **KOReader**, and send the highlights you make in
Bookx back, over your Wi-Fi. This works only with KOReader (on any device that runs it: Kobo, Kindle, Android, PocketBook
and others). Devices with their own firmware and no KOReader use the USB cable instead: see
[Devices and sharing](devices-and-sharing.md).

It is part of **Bookx sync (extended KOSync)**: Bookx’s built-in server plus a small KOReader plugin that Bookx can
copy to the device. Everything is under **main menu → Sync** and **Preferences → Sync**; see
[Bookx sync](progress-sync.md).

## Set it up

Follow the **Setup checklist** in *Preferences → Sync*:

1. **Turn on the Bookx sync server.** The address for the e-reader appears under the switch, ending in `/bookx`.
2. **Create your sync account** ([which user name and password?](#account)): type a user name and a password of your
   choice and press **Register**.
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

1. **Create it in Bookx**: *Preferences → Sync → 2. Create your sync account*, then **Register**. Or in KOReader:
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
- In **Notes & Highlights** a highlight that came from the device shows “KOReader (device name)”.
- If the same note was changed in both places, Bookx keeps yours and counts it in the sync line of the window.
- Highlights you make in Bookx are sent for the books the device has synced at least once (not for other books). On the
  device, **Bookx Sync → Notes from Bookx for this book** lists them. KOReader does not paint them on the page.
- Bookmarks are received and counted, but not added to the library (KOReader describes a bookmark as a place in its own layout).
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
