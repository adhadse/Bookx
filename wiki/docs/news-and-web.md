# News and Web

## Add from the Web

Click **+** → *Add from the Web* and paste an address.

- A **page address** becomes a readable book, to read later. Images are included.
- A **feed address** (RSS, Atom or JSON Feed) becomes a news book of its latest articles.

The new book lands in your library like any other and can be read, edited and sent to a device.

## Online catalogues

Menu → *Online Catalogues* browses OPDS catalogues. Project Gutenberg and Standard Ebooks are listed; the **+** button
adds others, such as a Calibre server (`http://host:8080/opds`) or another Bookx that shares its library. A user
name and password are only needed when the server asks for them; they are sent to that server only, and the password is
kept in the system keyring.

Open a catalogue, follow the folders (*Back* returns), search with the search entry, and press *Add* on a book. Bookx
downloads the best format on offer (EPUB, then KEPUB, AZW3, MOBI, FB2, CBZ, PDF) and imports it like any other file.
Long lists end with *Load more*. Limits: no sign-in pages other than basic authentication, no paid or borrowed books
(the DRM-protected ones are refused by the importer), and the built-in catalogues need network access.

## News subscriptions

Open **News Subscriptions** to have Bookx build a news book for you on a schedule. You find it in two places: the
**+** menu and on the **welcome page** of an empty library.

1. Press *Add*. Give it a **name** (the refresh button next to the name takes it from the feed).
2. Enter one or more **feed addresses**, one per line. A line starting with `#` switches a feed off. You can also
   import an **OPML** file.
3. Choose the number of **articles per feed**, and whether to download **full articles** (each article's web page
   instead of the feed text).
4. Choose the **schedule**: manual, hourly, daily, or weekly, with a time of day (`HH:MM`, in your local time) and,
   for weekly, a day.
5. Choose **issues to keep**. Older issues are removed from the library after a download.

Each download becomes a new book tagged *News*. *Download Now* and *Download All Due* are in the window. A failed
download is shown in the list and waits for the next slot instead of retrying every minute.

Subscriptions are stored in `news.json` next to the settings file.

!!! note "Limitations"
    - Scheduled downloads only run **while Bookx is open**. There is no background service.
    - Downloads need network access.
    - There are no scripted "recipes" like Calibre's; each subscription is a list of feeds.
    - Only the newest *Issues to keep* issues are kept; the older ones are removed by Bookx.
