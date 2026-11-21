# News and Web

## Add from the Web

<figure markdown="1">
![The Add from the Web dialog](assets/library-add-web-light.png#only-light)
![The Add from the Web dialog](assets/library-add-web-dark.png#only-dark)
<figcaption markdown="1">Add from the Web: paste an address; Bookx makes an e-book of the page.</figcaption>
</figure>


Click the **+** button at the top of the sidebar, choose *Add from the Web* and paste an address. Copying an address and pressing ++ctrl+v++ on Home opens the same window.

- A **page address** becomes a readable book, to read later. Images are included.
- A **feed address** (RSS, Atom or JSON Feed) becomes a news book of its latest articles.

The new book lands in your library like any other and can be read, edited and sent to a device.

## Online catalogues

<figure markdown="1">
![The Online Catalogues window](assets/library-catalogues-light.png#only-light)
![The Online Catalogues window](assets/library-catalogues-dark.png#only-dark)
<figcaption markdown="1">Online Catalogues: Project Gutenberg and Standard Ebooks are built in; add your own OPDS address with **+**.</figcaption>
</figure>


The **Library** menu in the header bar (next to *View options*) → *Online Catalogues* browses OPDS catalogues. Project Gutenberg and Standard Ebooks are listed; the **+** button
adds others, such as a Calibre server (`http://host:8080/opds`) or another Bookx that shares its library. A user
name and password are only needed when the server asks for them; they are sent to that server only, and the password is
kept in the system keyring.

Open a catalogue, follow the folders (*Back* returns), search with the **search button** in the header (or
<kbd>Ctrl</kbd>+<kbd>F</kbd>), and press *Add* on a book. While a page loads you can press *Cancel*; a page that cannot
be loaded offers *Try Again*. A password for an `http://` address is sent unencrypted: the dialog warns you. Bookx
downloads the best format on offer (EPUB, then KEPUB, AZW3, MOBI, FB2, CBZ, PDF) and imports it like any other file.
Long lists end with *Load more*. Limits: no sign-in pages other than basic authentication, no paid or borrowed books
(the DRM-protected ones are refused by the importer), and the built-in catalogues need network access.

## News subscriptions

<figure markdown="1">
![The News Subscriptions window](assets/library-news-light.png#only-light)
![The News Subscriptions window](assets/library-news-dark.png#only-dark)
<figcaption markdown="1">News Subscriptions: each feed with its schedule and how many issues to keep.</figcaption>
</figure>


Open **News Subscriptions** to have Bookx build a news book for you on a schedule. Click the **Library** button in the header bar (next to *View options*) and choose *Subscribe to News*; the **welcome page** of an empty library has the same button.

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

## Questions and answers

??? question "How do I save an article to read later?"
    **+** → *Add from the Web*, paste the address and confirm. The page becomes a book in your library, images
    included, and can be sent to your e-reader like any other.

??? question "How do I get a daily news book?"
    **Library** menu → *Subscribe to News* → *Add*. Enter the feed address, choose *Daily* and a time, and Bookx builds
    a new book each day while it is running. Each issue is tagged *News*.

??? question "Why did my news not download overnight?"
    Scheduled downloads only run while Bookx is open. Open it and press *Download All Due* in the News Subscriptions
    window.

??? question "How do I stop old issues piling up?"
    In the subscription set *Issues to keep*; older issues are removed after each download.

??? question "How do I browse Project Gutenberg or my Calibre server?"
    **Library** menu → *Online Catalogues*. Gutenberg and Standard Ebooks are listed; press **+** to add your own
    OPDS address.

??? question "Can I buy or borrow books there?"
    No. Paid and borrowed books and sign-in pages other than basic authentication are not supported.
