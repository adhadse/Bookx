# Reading

Open a book from the library. The reader draws the text itself with Pango and cairo, so there is no web view. The
header and navigation fade away after a few idle seconds so the page fills the window; moving the pointer brings
them back. Your position is remembered per book.

<div class="shots" markdown>

![The reader, with highlighted passages and the selection popover open on a note](assets/reader-light.png#only-light)
![The reader, with highlighted passages and the selection popover open on a note](assets/reader-dark.png#only-dark)

![The reader with the contents sidebar](assets/reader-contents-light.png#only-light)
![The reader with the contents sidebar](assets/reader-contents-dark.png#only-dark)

</div>

*Real screenshots.*

## Pages and layout

- **Two columns**, **single column** or **continuous scroll**, with real line-based pagination.
- A progress slider for the whole book and, under it, the **footer** with the page, the chapter and the time left
  (you choose what it shows, see [The footer](#the-footer)).
- Turn pages with ++left++ / ++right++, the mouse wheel, swipes or tap zones. ++ctrl+left++ / ++ctrl+right++ jump a
  chapter.
- Double-click the margin or press ++f11++ for fullscreen. Pinch or ++ctrl++ + wheel zooms.
- The sidebar button at the left of the header bar (or ++f9++) opens the book, its table of contents, bookmarks and notes
  (see [The sidebar](#the-sidebar)). The magnifier at the right of the header bar, or ++ctrl+f++, searches the whole
  book. The bookmark button next to it adds a bookmark at your place (++ctrl+d++). Links and footnotes have
  a *Back* button.
- ++escape++ puts away the first thing that is open: the selection popover, a handle you are dragging, the selection, the
  sidebar, full screen, and only then returns to the library. ++ctrl+r++ reloads the book.
- **Removing is not asking.** Removing a highlight (with its note), a bookmark or a word of your vocabulary happens at
  once and a message with **Undo** stays for ten seconds. Opening a book that is damaged or missing shows a page that
  says why, with **Back to Library**; a big book shows *Opening Book* while its first chapter is laid out.

## The reader menu

<figure markdown="1">
![The reader menu with the themes row](assets/reader-menu-light.png#only-light)
![The reader menu with the themes row](assets/reader-menu-dark.png#only-dark)
<figcaption markdown="1">The Main Menu: *Go to*, *Speed Reading*, *Book Map*, *Terms in This Book*, *Footer Statistics* and *Reading Ruler* (a check item), *Open Book* and *Reload*, then *Preferences*, *Keyboard Shortcuts*, *Help* and *About Bookx*.</figcaption>
</figure>


The **Main Menu** (the hamburger button at the far right of the header bar, or ++f10++) has thirteen entries in three groups and no submenus. *This
book*: *Go to*, *Speed Reading*, *Book Map*, *Terms in This Book*, *Footer Statistics* and *Reading Ruler*, which has a
check mark while the ruler is on. *File*: *Open Book* and *Reload*. Then *Preferences*, *Keyboard Shortcuts*, *Help*
(the online manual, ++f1++), *Report a Problem* (the report starts with what the reader did last) and *About Bookx*. The keys are written next to the entries. (*Properties* of the book is a
button in the sidebar.) Everything about how the text looks, the themes and the size included, is in the **Text
Appearance** button a little to its left in the header bar (see [Text appearance](#text-appearance)).

**Narrow windows.** In a window narrower than about 600 px (a phone, or a half-screen window) the header gives up
*Fullscreen* and *Read Aloud*, and *Text Appearance* moves into the menu: they are its first three entries, and the
page-jump buttons show icons instead of words. PDFs and comics have the same Main Menu (*Go to Page*, *Properties* and
the same last four entries) and give up *Fullscreen* the same way.

## The sidebar

++f9++ (or the sidebar button at the top left) opens the sidebar over the page; ++f9++, ++escape++ or a click on the page
puts it away again. It lies **over** the page, so the text does not move or lay itself out again, and it opens at once
even in a book with thousands of contents entries and hundreds of highlights: only the lines you can see are built.

![The reader's sidebar with the table of contents](assets/reader-sidebar-contents-light.png#only-light)
![The reader's sidebar with the table of contents](assets/reader-sidebar-contents-dark.png#only-dark)

At the top is the book: a small cover, the title, the author and a **Properties** button (path, identifiers, size and
so on). Under it the tabs **Contents**, **Bookmarks** and **Notes** switch the page; the list you are on takes the
keyboard (arrow keys, ++enter++ to go there).

- **Contents**: the chapters, indented by level; the chapter you are reading is marked and scrolled into view.
- **Bookmarks**: where each bookmark is, with its date and a button to remove it. Add one with ++ctrl+b++ or ++ctrl+d++.
- **Notes**: every highlight of the book, grouped under its chapter, in the order of the book.

![Notes and highlights in the sidebar](assets/reader-sidebar-notes-light.png#only-light)
![Notes and highlights in the sidebar](assets/reader-sidebar-notes-dark.png#only-dark)

Each highlight shows its quote **drawn the way it is on the page** (background, underline, strikethrough or squiggly, in
its colour), the note under it with a pencil, and the date. Click it to go there. The **⋮** button (or a right click, or
++menu++ / ++shift+f10++ on the keyboard) opens a small menu: a row of icons for *Edit note*, *Copy text* and *Delete*,
then the five **colours** and the four **styles** as round and "Aa" buttons, the current ones marked (arrow keys move
between them). The sidebar always uses the colours of the app, light or dark as your desktop is, whatever reading theme
the book has.

The search box at the top finds highlights by words of the quote, the note or the chapter (every word must occur).
**Filter** narrows the list by colour, by style and to highlights **with a note**; the line under the box says
"12 of 40 highlights" and *Clear filters* shows them all again.

### Go to and search

<figure markdown="1">
![The Go to dialog](assets/reader-goto-light.png#only-light)
![The Go to dialog](assets/reader-goto-dark.png#only-dark)
<figcaption markdown="1">Ctrl+G: type a page, a percentage, a location or a chapter name.</figcaption>
</figure>

<figure markdown="1">
![Search in the book with a list of matches](assets/reader-search-light.png#only-light)
![Search in the book with a list of matches](assets/reader-search-dark.png#only-dark)
<figcaption markdown="1">Ctrl+F: every match with its chapter; Enter or a click jumps to it and highlights it.</figcaption>
</figure>


The search popover opens from the magnifier in the header bar and fits the window (it is narrower and shorter in a small window). The line under the field counts
the matches while it searches ("Searching, 12 so far"); a search with no match shows a *No Results* page.

Three buttons under the field change what a search means: **Words** (whole words only, so *cat* does not find *concat*),
**Aa** (match case) and **.\*** (a regular expression, for example `Chap.er [0-9]`). A regular expression that is not valid says
so under the field. The choices are kept until you change them.

To jump somewhere, open the Main Menu and choose *Go to* (or press ++ctrl+l++ or ++ctrl+g++). It asks where to go. The field says *Page, percent, location or chapter*;
the line under it tells what the book has and, as you type, where the text leads. A text that does not work turns the
field red and explains why. Type:

| You type | Where it goes |
|---|---|
| `45` | page 45 of the book (the page count of the current layout, which depends on the text size) |
| `iv` or `45` in a book with printed page numbers | that **printed page** (see below) |
| `37%` | 37 % of the book |
| `loc 1200` or `@1200` | that location (a location is about 128 characters of text) |
| `ch 3` | the third entry of the table of contents |

The line under the field says where it leads before you press *Go*: *Page 45 of 312*, *37 percent*, *Chapter 3 of 12*,
*Location 1200 of 3100*; and why a text does not work. (`loc` and `ch` still work as shortcuts; the field does not
advertise them.)

**Printed pages.** Some EPUBs carry their paper page numbers (an EPUB *page-list*). Then the footer shows *Page iv*
instead of "12 of 340", and *Go to* takes those labels. Books without a page-list keep the layout page count.

The *Font* list in the Text Appearance popover shows only the families installed on your computer (plus Serif, Sans and
Monospace), so a font that is not there cannot be picked by mistake.

### Back and forward

Following a link, choosing a table-of-contents entry, a search result, a bookmark or a highlight, dragging the slider
or using *Go to* remembers where you were. **Previous Position** (++alt+left++, the button at the top left that appears
after a jump) returns there; **Next Position** (++alt+right++) goes to where you came back from. A new jump forgets the forward list. Dragging the slider
counts as one jump.

### Images

Click a picture in the middle of the page to open it in the image viewer: the outer edges still turn pages. The viewer
zooms with the buttons, ++plus++ / ++minus++, ++ctrl++ + wheel, a pinch or a double click (fit ↔ 100 %); ++0++ fits
the window and ++1++ shows the actual size. Drag, scroll or use the arrow keys to pan a zoomed picture. **Copy**
(++ctrl+c++) puts the picture on the clipboard and **Save** (++ctrl+s++) saves the original file of the book. ++escape++ closes the
viewer. Pictures in fixed-layout books and comics are not clickable (taps turn pages there); text-flow pictures
inline in a line of text are not either.

### Book map

<figure markdown="1">
![A close-up of the book map](assets/book-map-detail-light.png#only-light)
![A close-up of the book map](assets/book-map-detail-dark.png#only-dark)
<figcaption markdown="1">Close-up of the book map: reading progress, highlights and the preview of a place.</figcaption>
</figure>


*Book Map* (++m++) shows every page of the book as a grid of small cells, in a window of its own. The page numbers are
the printed ones when the book has a page-list, otherwise the pages of the current layout.

<div class="shots" markdown>

![The book map: pages as small blocks with read bars, highlight lines in their colours, a note, bookmarks, numbered places and the preview card](assets/book-map-light.png#only-light)
![The book map: pages as small blocks with read bars, highlight lines in their colours, a note, bookmarks, numbered places and the preview card](assets/book-map-dark.png#only-dark)

</div>

*Real screenshots.*

- **Bars** under the pages you have read: the taller the bar, the longer the page was on screen.
- A soft line marks where a **chapter** begins, with the chapter's title above it when single pages are shown. The page
  you are on has an **accent outline and a small pill** under it.
- **Highlights** show inside the page's block as a thin **one-pixel line in their own colour** (yellow, green, blue, red,
  purple). Several highlights on one page are separate lines, a pixel apart, in the order they come in the
  book, from edge to edge of the block; if they do not all fit the map adds "+N". A **pencil** in the highlight's colour
  **under the block** is a note (point at it to read the note in the card) and a **flag** is a bookmark. The **numbered circles** are the places you jumped from: **1** is where *Back*
  (++alt+left++) goes next, then 2, 3 …
- The **legend** at the bottom says what the marks mean; the button at the top right hides it.
- Point at a page, or **press and drag** through the whole book: a small, solid card follows the pointer (and stays inside
  the map) with a **thumbnail of the page itself**: one page, laid out exactly as the reader shows it, in your reading theme
  and with its highlights, then the chapter, the page and how far into the book it is. The pages around the pointer are
  made in the background when the map opens, so moving over them is instant; until a page is ready the nearest one that
  is stands in, dimmed. Thumbnails are kept in the cache folder (up to 64 MB), so the next time the map opens they are
  there at once. Point at a **pencil** to read its note in the card. Let go (or click, or press ++enter++) to go there.
- The **mouse wheel** scrolls the map, ++ctrl++ + wheel changes how much of the book you see (whole book, sections,
  single pages); the two buttons at the top do the same. The dots on the right show the zoom level and the bar the part
  of the book in view. ++left++ / ++right++ / ++up++ / ++down++, ++page-up++ / ++page-down++, ++home++ and ++end++ move
  the cursor. ++escape++ closes the map.

The map, its preview card and the legend follow the application's light or dark style, whatever the reading theme of the book is. The jump can be undone with *Back*. Highlights are placed approximately inside their chapter. The time per page is
kept on this computer (`dwell.json` next to the settings) and is not shared.

### Themes and your own themes

The themes (follow the system, **light**, **sepia**, **dark** and **black**) are swatches at the top of the *Text Appearance* popover (the text-format button in the header bar). A
**+** swatch next to them makes a **custom theme**: pick the text and page colour, see the preview (with a warning when
the contrast is too low to read), name it and **Save Theme**. Your themes get their own swatch with a small **pencil**
to edit or delete them; the editor also **exports** and **imports** themes as JSON. Themes are shared by all your books.

### Themes and colours

<figure markdown="1">
![A page in the Light theme](assets/reader-theme-light-light.png#only-light)
![A page in the Light theme](assets/reader-theme-light-dark.png#only-dark)
<figcaption markdown="1">Light</figcaption>
</figure>

<figure markdown="1">
![A page in the Sepia theme](assets/reader-theme-sepia-light.png#only-light)
![A page in the Sepia theme](assets/reader-theme-sepia-dark.png#only-dark)
<figcaption markdown="1">Sepia</figcaption>
</figure>

<figure markdown="1">
![A page in the Dark theme](assets/reader-theme-dark-light.png#only-light)
![A page in the Dark theme](assets/reader-theme-dark-dark.png#only-dark)
<figcaption markdown="1">Dark</figcaption>
</figure>


Whichever theme you pick, **More Text Options** (a button in the Text Appearance popover) has a
**Colours** section with three modes:

- **Theme**: the colours of the chosen theme.
- **Original**: the text and page colours the book itself asks for.
- **Custom**: pick your own text and page colours.

The editor's live preview has the same control (the palette button above the preview), see [Editing](editing.md).

### Text appearance

<figure markdown="1">
![The Text appearance popover](assets/reader-text-settings-light.png#only-light)
![The Text appearance popover](assets/reader-text-settings-dark.png#only-dark)
<figcaption markdown="1">Text appearance: font, paragraph spacing, optimal line breaking, hyphenation, columns, line spacing, margins and justification.</figcaption>
</figure>


The **Text Appearance** button (the text-format button in the header bar, next to the magnifier) keeps the basics in a small popover: the **theme swatches**, the
**text size** (smaller, bigger, back to 100 %), the **font**, **columns** (two, single, scroll), **line spacing** and
**margins** with their values written next to the sliders, and **justify text**.

The **More Text Options** button in that popover opens a window with the rest:

- **Colours**: *Theme / Original / Custom* and the colour pickers.
- **Text**: hyphenate words, paragraph spacing, optimal line breaking, the hyphenation language, **text weight**,
  **word spacing**, **text contrast**, **kerning**, **widows and orphans** (the least lines of a paragraph left at the
  bottom or carried to the top of a page, 1 to 4; the default is 2), **Book's own styling** and **Book's fonts and
  sizes**. Each row has its own switch: while it is off the default applies.
- **Advanced and custom CSS** (collapsed): overrides of line height, margins, font and justification, and your own
  style sheet that is applied after the book's.
- **This book**: *Use as Default for New Books* and *Reset This Book to Defaults*, each with a line that says what it does.

**Every change is kept for the open book** - there is nothing to save, and other books are not touched. New books start
from your defaults:

- **Use as Default for New Books** makes the current look (including the tweaks) the starting point of books you have
  not changed yet.
- **Reset This Book to Defaults** forgets this book's own look.

(Older named *reading profiles* were replaced by this: a profile pinned to a book became that book's own look; the
others were saved to `reading-profiles-backup.json` next to the settings.)

*Preferences → Reading → Use the book's own styling* applies the book's CSS classes (italic, bold, underline, small caps,
monospace, alignment, hidden text, first-line indents, paragraph spacing and the fonts and relative sizes the book asks
for) on top of the plain structure. Small capitals are drawn as real small capitals. The **Book's own styling** row in
More Text Options overrides the preference for one book (on or off), and **Book's fonts and sizes** keeps your font
while the rest of the styling stays. Tables are still shown as rows of cells separated by bars, and positioned text is
not reproduced.

### Reading aids

*Preferences → Reading → Reading aids* has the settings of a few small helpers:

- **Page-turn animation**: *None*, *Slide* or *Fade*. It is off whenever your system asks for reduced motion and in
  e-ink mode, whatever you choose here.
- **Auto-turn**: *Auto-Turn Pages* in the command palette turns a page every few seconds (the interval is
  in Preferences). It waits while a menu or a selection is open or the book is read aloud, starts the time over when you
  turn a page yourself and stops at the end of the book.
- **Evening schedule**: between two times the page gets a warm tint (blue is taken out), growing over 30 minutes
  before the evening and fading after it, and the dark theme is used when the reading theme is *Automatic*.
- **Copy with a citation**: ++ctrl+shift+c++ copies the selection with its source. The template uses `{text}`,
  `{title}`, `{author}`, `{chapter}`, `{page}`, `{date}` and `{isbn}`; a separator in front of an empty field goes with
  it. A switch makes every copy a citation.
- **Link hints**: *Show Link Hints* (++ctrl+semicolon++) puts a letter on every link of the page; type the letter to
  follow it, anything else puts the hints away.
- **Page turner keys**: the media keys (*Next*, *Previous*, *Forward*, *Rewind*), the keypad ++plus++, ++minus++ and
  ++enter++ and ++f7++ / ++f8++ of Bluetooth page turners and presentation remotes turn pages. Add your own as
  `KeyName=action` (actions: `next-page`, `prev-page`, `next-chapter`, `prev-chapter`, `toggle-chrome`); a key with no
  action frees a default.
- **Archive finished books**: off by default; when on, a book you read to its end is added to a shelf called *Archive*.

Dragging the progress slider shows the chapter and percentage under the pointer. The **Contents** list can fold:
a section with sub-sections has an arrow, and *Collapse All* / *Expand All* sit above the list. The chapter you are
reading is always shown. Moving a handle of a saved highlight changes the highlight itself.

## The footer

Under the page the footer shows reading statistics. Right-click it, or choose **Footer Statistics** in the Main Menu, to
open the *Footer Statistics* window: the preset, the items (each with up and down buttons), how the footer looks and
*Only for this book*, in lists; every change shows in the footer at once. The same settings are in *Preferences →
Reading → Footer Statistics*.

![The footer settings](assets/reader-footer-settings-light.png#only-light)
![The footer settings](assets/reader-footer-settings-dark.png#only-dark)

- **Items**: the page (*12 of 340*, or the printed page), the page in the chapter, the percent read, the time left in
  the chapter and in the book, your reading speed, the time of day, the battery (when the computer has one), the chapter
  title, the book title, the author, the progress bar, the chapter you are in, the time you have read in this session
  and today. Switch each on or off and move it up or down.
- **Presets**: *Minimal* (just the page), *Standard* (the page at the left, the chapter and the time left at the right,
  with the progress bar; this is how the footer always was) and *Everything*.
- **Position**: either the first item at the left and the rest at the right, or **one line in the middle** of the
  page: between the two columns of a two-column page, in the middle of the column of a single-column page.
- **Separator**, **text size** and **opacity**.
- **Only for this book** gives the open book a footer of its own; the others keep the common one.

Changes apply at once, in every open book, even while Preferences is open (the footer of a book with its own settings keeps them).

Times left use your own reading speed once the statistics know it (about 230 words a minute before that) and are
estimates. Items that have nothing to say (no battery, no speed yet) are left out.

## Selecting text

A double click, a long press with a finger or a drag with the mouse selects text. Two blue **handles** appear at its ends
(like on a phone): drag the one at the start or the end to make the selection shorter or longer. With a finger the end
snaps to whole words; with the mouse it moves by character, and holding ++ctrl++ snaps to words. A handle dragged past
the other end swaps them. The action buttons (colours, note, copy, dictionary, Wikipedia, translate) stay near the
selection and come back when you let go; the handles disappear with the selection (++escape++ or a click on the page).
Handles work in two columns and in vertical text; in e-ink mode they are black and nothing moves.

![Handles at the ends of a selection](assets/reader-handles-light.png#only-light)
![Handles at the ends of a selection](assets/reader-handles-dark.png#only-dark)

## Typography

- **Hyphenation** in the language of the book: switch *Hyphenate words* on in **More Text Options** (for this book) or in
  *Preferences → Reading* (for every book), for justified text without rivers.
- **Optimal line breaking** (Knuth-Plass, as in TeX) in justified text. Pango's own breaking is the fallback for
  right-to-left text, or when you switch optimal breaking off in **More Text Options**.
- **Drop caps and floats** from the book's CSS: `::first-letter { float: left }` and floated images, with the text
  wrapping around them.

## Footnotes

Following a link to a footnote or endnote shows the note in a small popup where you are, without its number and
back arrow, with a **Go to Note** button if you want to jump there. A link is treated as a note when its target is
marked as one in the book (`epub:type="footnote"`, `role="doc-endnote"` and so on), is a list item or has an id such
as `fn1`, and the text is short. Links to chapters and headings are followed as before. Switch the popups off in
*Preferences → Reading → Links*.

## Reader focus

<figure markdown="1">
![The reading ruler dimming all lines but one](assets/reader-ruler-light.png#only-light)
![The reading ruler dimming all lines but one](assets/reader-ruler-dark.png#only-dark)
<figcaption markdown="1">The reading ruler keeps one line bright and dims the rest.</figcaption>
</figure>

<figure markdown="1" class="clip">
<video class="only-light" data-clip muted loop playsinline controls preload="none" width="960" height="576" poster="../assets/reading-ruler-demo-poster-light.webp" aria-describedby="reading-ruler-demo-text"><source src="assets/reading-ruler-demo-light.mp4" type="video/mp4"><source src="assets/reading-ruler-demo-light.webm" type="video/webm">Your browser cannot play this video: <a href="assets/reading-ruler-demo-light.mp4">download it</a>.</video>
<video class="only-dark" data-clip muted loop playsinline controls preload="none" width="960" height="576" poster="../assets/reading-ruler-demo-poster-dark.webp" aria-describedby="reading-ruler-demo-text"><source src="assets/reading-ruler-demo-dark.mp4" type="video/mp4"><source src="assets/reading-ruler-demo-dark.webm" type="video/webm">Your browser cannot play this video: <a href="assets/reading-ruler-demo-dark.mp4">download it</a>.</video>
<figcaption id="reading-ruler-demo-text" markdown="1">The page is dimmed except for a bright band that moves down the lines with Alt+Down and back up with Alt+Up, then follows the pointer down and up the page. A short video without sound: it plays by itself unless your system asks for less motion, and its controls pause it.</figcaption>
</figure>


While you read, the header and the bottom panel get out of the way: a few seconds after your last interaction they fade
and only the page is left, and they come back as soon as you move the pointer, press a key or open a menu (they never
fade while you use them, and not when the pointer rests on them or at the top or bottom edge). The page does not move
when they come and go.

While [read aloud](read-aloud.md) or speed reading is **playing** this happens faster: **1.5 seconds** after you let go
of the mouse nothing is left but the page and the open card(s); the title and the status line go too. Move the pointer,
press a key or press Esc and the bars are back, with the page number on the left and the chapter and the time to
finish on the right. Paused, the reader behaves as it always does (the bars fade after 4 seconds and keep the title and
the status line). With an e-ink display the bars stay.

## Speed reading

<figure markdown="1">
![The speed reading card](assets/reader-speed-card-light.png#only-light)
![The speed reading card](assets/reader-speed-card-dark.png#only-dark)
<figcaption markdown="1">Speed reading: one word at a time at the speed you set; Space pauses.</figcaption>
</figure>

<figure markdown="1" class="clip">
<video class="only-light" data-clip muted loop playsinline controls preload="none" width="960" height="576" poster="../assets/speed-read-demo-poster-light.webp" aria-describedby="speed-read-demo-text"><source src="assets/speed-read-demo-light.mp4" type="video/mp4"><source src="assets/speed-read-demo-light.webm" type="video/webm">Your browser cannot play this video: <a href="assets/speed-read-demo-light.mp4">download it</a>.</video>
<video class="only-dark" data-clip muted loop playsinline controls preload="none" width="960" height="576" poster="../assets/speed-read-demo-poster-dark.webp" aria-describedby="speed-read-demo-text"><source src="assets/speed-read-demo-dark.mp4" type="video/mp4"><source src="assets/speed-read-demo-dark.webm" type="video/webm">Your browser cannot play this video: <a href="assets/speed-read-demo-dark.mp4">download it</a>.</video>
<figcaption id="speed-read-demo-text" markdown="1">One word after the other lights up on the page, with a red letter to rest your eye on, while a small card shows the word and the speed. A short video without sound: it plays by itself unless your system asks for less motion, and its controls pause it.</figcaption>
</figure>


Main menu → *Speed Reading* (or the command palette) reads on the page itself: the word to read has a broad
highlight and the letter your eye should rest on a strong red one, and they move through the text by themselves; the page
turns to keep the word in view and the next chapter follows the last one. A small card (at most about 360 × 120 px) floats over
the page with play/pause, previous / next sentence, slower / faster (steps of 25 words per minute) and the progress of
the chapter. Drag it by any part of its background to move it anywhere in the window (double-click puts it back; the
place is kept as a share of the window size; the two cards never overlap, see [Read aloud](read-aloud.md)); the **X** in the top right corner (*Close speed reading*, or **Esc**
while the card has the focus) stops speed reading and closes only this card.
It starts on the page you have open, paused. ++space++ starts and pauses, ++left++ / ++right++ step a word, ++up++ /
++down++ change the speed and ++escape++ closes it. Commas, sentence ends, paragraph ends and long words take a little
longer. The speed is remembered. Speed reading and [read aloud](read-aloud.md) are independent cards that can both be
open: starting one pauses the other (it stays open, at its place). When you leave the book and come back (or restart
Bookx) the card is where it was, at the same word and speed, paused; a card closed with the X stays closed.

## Dictionaries and Wikipedia

<figure markdown="1">
![The popover over a selected word](assets/reader-selection-popover-light.png#only-light)
![The popover over a selected word](assets/reader-selection-popover-dark.png#only-dark)
<figcaption markdown="1">Select a word: highlight colours and styles, copy, note, dictionary, Wikipedia and translate.</figcaption>
</figure>


Double-click a word, or select text, to look it up.

- **Dictionary**: offline StarDict dictionaries, see [Dictionaries](#dictionaries) below.
- **Wikipedia**: an inline summary of the selection with the **lead picture** of the article above it, when it has
  one. The picture is fetched after the text (a placeholder shows meanwhile), kept in a small cache folder (at most
  24 MB, the oldest pictures go first) and shown from there when you are offline (`BOOKX_OFFLINE`); a picture that
  cannot be loaded just leaves out its frame.
- **Translate**: an inline translation with a language picker. Wikipedia and translation need network access.

![A Wikipedia look-up with the picture of the article](assets/reader-wikipedia-light.png#only-light)
![A Wikipedia look-up with the picture of the article](assets/reader-wikipedia-dark.png#only-dark)

Looked-up words are added to your [vocabulary](#statistics-and-vocabulary). A word that is in no dictionary (a name,
jargon, an invented word, an abbreviation) can be added with **your own definition**, see
[My own words](#my-own-words-and-definitions).

### Dictionaries

<figure markdown="1">
![A dictionary result in the popover](assets/reader-dictionary-light.png#only-light)
![A dictionary result in the popover](assets/reader-dictionary-dark.png#only-dark)
<figcaption markdown="1">The dictionary result opens in the same popover; the word is saved to your vocabulary.</figcaption>
</figure>


A look-up uses **StarDict dictionaries on your computer**, so it works offline. You do not have to find one yourself:

- **In the look-up popover.** If there is no dictionary for the language of the book (its `dc:language`; `en-GB` and
  `pt-BR` count as `en` and `pt`; without one Bookx guesses from the text), the popover says *No dictionary for German
  yet* and offers **Download dictionary (7 MB)**. One click downloads and installs it, shows the progress, and then
  shows the definition of the word you looked up in the same popover. *Choose another* opens Preferences →
  Dictionaries, *Learn more* opens this page.
- **By itself.** With *Preferences → Dictionaries → Download dictionaries automatically* on (the default), adding a book in a language that has no dictionary (import, Add from the Web, news,
  device) starts a download in the background, and so does opening such a book. Each language is one job in the
  **Background Jobs** panel (progress, cancel, log), with a toast *Dictionary for German ready* at the end. Books that
  were already in your library when the feature first ran do not start downloads by themselves; opening one does.
  Offline, nothing is started and nothing blocks the import: the language waits and is tried again later (after a
  failure the pauses grow from one minute to six hours). A dictionary bigger than 50 MB is never downloaded
  automatically without asking; the button in the popover is your consent. (The dictionaries in the catalogue are 0.3
  to 22 MB.)
- **Which dictionaries.** One catalogue ships with Bookx (`data/dictionaries.json`) with a dictionary for English,
  Spanish, French, German, Italian, Portuguese, Russian, Dutch, Polish, Turkish, Swedish, Czech, Japanese, Chinese,
  Arabic, Hindi, Korean, Greek, Hebrew and Ukrainian: for English the English definitions of Wiktionary (and
  Webster's 1913, public domain, as another choice), for the other languages the Wiktionary dictionary of that
  language with English definitions (headword in the language of the book). All are free to share: Wiktionary data
  is CC BY-SA 3.0 / GFDL, WikDict CC BY-SA 4.0. Each entry has the licence, size, SHA-256 of every download and
  several origins, tried in order.
- **Your own.** StarDict dictionaries (folders with `.ifo`, `.idx`, `.dict` or `.dict.dz`, optional `.syn`) put into
  `~/.local/share/bookx/dictionaries/` (*Open Folder* in the settings) are used too. A dictionary whose name says
  which language it is for (*German-English*) counts for that language, one that does not counts for every language,
  so Bookx does not offer a download next to your own.
- **Remove** a downloaded dictionary with the bin button in the settings: it disappears at once and a message with **Undo** stays for 8 seconds; the files are deleted when it goes away. A download in progress shows *Downloading… 3 MB of 12 MB* in the *Get a dictionary* row, and a failure says *Could not download* and why. Downloads are checked against the SHA-256 of
  the catalogue and unpacked safely (no paths outside the folder, size limits).
- **Privacy.** The only hosts Bookx contacts for dictionaries are the ones named in the catalogue entry:
  `github.com` / `raw.githubusercontent.com` (the Wiktionary dictionaries of Vuizur, and Webster 1913) and
  `download.wikdict.com` (WikDict). Nothing about you or your book is sent except what any download sends (your
  address, the file name). With `BOOKX_OFFLINE` set (or the switch off) no dictionary is downloaded.

### Dictionary order, web sites and history

Preferences → **Dictionaries** also has three groups for the look-up:

- **Dictionary order**: every installed dictionary with **Move Up** and **Move Down**. The first one that knows the word
  answers first, then the others. The order is kept in a small file (`priority.txt`) in the dictionaries folder; a
  dictionary you add later is asked last.
- **Web look-up sites**: the look-up popover has a **Web** menu with Wiktionary, a web search and Wikipedia, and with your
  own sites. **Add Site** takes a *name* and an *address* in which `{word}` stands for the selected text, for example
  `https://dictionary.example.com/search?q={word}`. Only `http` and `https` addresses are accepted, the site name must be
  written out (the text you select never chooses the site), and the text is encoded before it is added. The site opens in
  your browser; Bookx sends nothing itself.
- **Look-up history**: the words you looked up, newest first (up to 200), kept on this computer. **Show** (or **Web → History** in the
  popover) lists them; the bin removes one word, **Clear** empties the list and a message offers **Undo**. Switch
  *Keep a history of look-ups* off to stop recording.

## Annotations and highlights

<figure markdown="1">
![The popover of a highlight with its note](assets/reader-note-popover-light.png#only-light)
![The popover of a highlight with its note](assets/reader-note-popover-dark.png#only-dark)
<figcaption markdown="1">A highlight with a note: the colours, the four styles, and the note itself.</figcaption>
</figure>


Select text and a small popover appears next to it: click one of the **5 colours** to highlight it, and pick one of the
**4 styles** (background, underline, strikethrough, squiggly). The pencil button adds a note, which is saved when you
click away. Click an existing highlight to open the same popover again: change its colour or style there, edit the note,
or remove it with the bin. Add a bookmark with the bookmark button at the left of the header bar (or ++ctrl+d++). Highlights, notes and bookmarks are listed in [the sidebar](#the-sidebar), where you can also
search and filter them and change a highlight's note, colour or style.

Highlights, notes and bookmarks that your e-reader sends over Wi-Fi appear on the open page while you read, and the page offers to continue where the e-reader is when it is further along ([live updates](koreader-sync.md#live)).

Export them from the library (right-click → *Export Annotations*) to JSON, Markdown or plain text.
[Send to a device](devices-and-sharing.md#send-to-device) adds them to the file you send (switch off in the Device window if you do not want that).

## Statistics and vocabulary

<figure markdown="1">
![The Add to Vocabulary dialog](assets/reader-vocab-add-light.png#only-light)
![The Add to Vocabulary dialog](assets/reader-vocab-add-dark.png#only-dark)
<figcaption markdown="1">Add to Vocabulary: your own term, your definition, a note, tags, and whether it applies to this book or all.</figcaption>
</figure>


<figure markdown="1">
![The Reading Statistics window](assets/library-statistics-light.png#only-light)
![The Reading Statistics window](assets/library-statistics-dark.png#only-dark)
<figcaption markdown="1">Reading Statistics: time today, this week and all time, your streak, books finished and your goals.</figcaption>
</figure>

<figure markdown="1">
![The Vocabulary flashcards](assets/library-vocabulary-words-light.png#only-light)
![The Vocabulary flashcards](assets/library-vocabulary-words-dark.png#only-dark)
<figcaption markdown="1">Vocabulary: the words you looked up come back as flashcards; *Show Answer* turns the card.</figcaption>
</figure>


- **Reading Statistics** (in the **Library** menu of the library's header bar): time today, this week and this month, streaks, reading speed, a 7-day chart and
  your most-read books. It also holds your **goals** (a switch for *Daily goal* and for *Yearly goal*; switching one on shows the amount and a progress bar, off means
  no goal), a **calendar of the year** that shades the days you read, and a **review of the year** (time, days read,
  pages, books finished, longest streak, best day).
- **Vocabulary** (also in the **Library** menu): two pages. **Review** is the spaced-repetition flashcards of the words you looked up.
  **Words** is the list of everything in your vocabulary, with a search, the filter *All / Mine / From dictionary*, and
  the buttons to add, edit and remove terms, paste or import a glossary, and export. See
  [My own words and definitions](#my-own-words-and-definitions).

## My own words and definitions

A term does not have to be in any dictionary to go into your vocabulary. Names, jargon, invented words and
abbreviations can be saved **with your own definition**, and you see that definition again wherever the term appears in
the book.

**Adding a term.** Select a word or a phrase (any selection, several words are fine), then choose **Add to
Vocabulary**: the **+** button of the selection popover, the key **V** while text is selected, *Add to Vocabulary* in the
command palette (`Ctrl+Shift+V`), or **Add my own definition** in the look-up popover (it is there whether or not the
dictionary has an entry). A small dialog opens with:

- the **Term** (editable; white space is made single and punctuation around it is dropped),
- **Your definition** (several lines),
- an optional **Note** and **Tags**, and the **Language** (the language of the book),
- **Applies to**: *This book* (the default), *All books*, or *This series* (when the book belongs to one),
- the sentence you met it in, kept automatically with the place in the book.

**Save** (or `Ctrl+Enter` from anywhere in the dialog) stores it. Everything is reachable with the keyboard: select
with `Shift+arrows`, press `Enter` and then `V`, type the definition, `Tab` to the other fields, `Ctrl+Enter`.

**In the look-up popover** your definition comes first, labelled **Your definition**, above the dictionary results,
and the button says **Edit my definition**. It works offline and with no dictionary installed.

**In the text.** Every occurrence of a saved term in the book gets a **dotted underline** in the reading colour
(it follows the theme and e-ink mode). Click or tap one to see your definition in a small popover with **Edit**,
**Remove** and **Open in Vocabulary**; to select the word instead, double-click it as usual. Matching ignores case,
needs whole words (`art` is not found in `part`; there are no inflections, so `walk` does not mark `walked`), finds
phrases (`New York`) and works with any script and with hyphenated words. A term marked inside a highlight opens the
highlight's popover instead. Turn the marks off in *Preferences → Reading → Vocabulary → Mark vocabulary terms in
the text* (on by default).

**Terms in this book** (reader menu, or the command palette) lists what you defined for the book and the words you met
in it; the arrow button of a term goes to the place where you met it.

**The Words page.** Terms with your own definition carry a **Mine** badge. *All / Mine / From dictionary* filters, the
search looks in words, definitions, notes and tags. **Add Term**, the pencil (edit; on a word from a dictionary it
adds your own definition) and the bin work on a row; a removed term can be brought back with **Undo**. **Add Term**
(the **+** button) and the **More** menu (the three dots) are in the header of the window. **More** has:

- **Paste a Glossary**: one term per line as `term; definition` (also a tab, or `term — definition`); the dialog counts
  the terms it found before you add them,
- **Import a CSV File**: `term;definition` with an optional note and tags column, quoted fields understood (a header
  row is skipped),
- **Export Your Definitions as CSV or JSON**, **Export Everything as Anki Cards** (the text file Anki imports; your
  definition and note are on the back) and **Export as a StarDict Dictionary**.

**My Vocabulary as a dictionary.** *Preferences → Dictionaries* lists **Your vocabulary** first, before the
downloaded ones. It is built in and cannot be removed; its *Export* button writes it as a StarDict dictionary, JSON or
CSV for other apps.

Limits: one definition per term (writing the same term again edits it; its *Applies to* is the last one you chose),
the marks are found in the text of a chapter, not in images, and the vocabulary is kept on this computer (it is not
part of the library backups and not synced).

## Read aloud

The speaker button in the header bar opens a small popover with play / pause, *Voice Settings* and *Save Chapter as
Audio* / *Save Book as Audio*. Play reads the book sentence by sentence and highlights the sentence; a player card then
floats over the page with the skip buttons, the speed, the voice and the sleep timer (pitch is in *Voice Settings*). It
goes on with the next chapter, and starts at the selection or the top of the page.

The voice can be a natural one that runs on your computer (Kokoro, Piper), a voice server you run (XTTS v2, Fish
Speech, anything OpenAI-compatible). Read aloud has a music-player style card with seek bar, speed and sleep timer. See [Read aloud](read-aloud.md) for setup,
licences and limits.

## E-ink mode and touch

*Preferences → Reading* has the settings for tablets and e-ink screens:

- **Touch**: swipe to turn pages, long-press to select a word, configurable **tap zones** (edges, thirds,
  left-handed, wide, off) and large touch controls.
- **E-ink mode**: pure black on white, no animations, bars that stay, wide tap zones.

<figure markdown="1">
![E-ink mode](assets/reader-eink-light.png#only-light)
![E-ink mode](assets/reader-eink-dark.png#only-dark)
<figcaption markdown="1">E-ink mode, captured on a normal screen: the colours show how it looks, not how an e-ink panel renders it.</figcaption>
</figure>

## Fixed layout and right-to-left text

- **Fixed-layout EPUB** (pre-paginated): one page at a time, pictures scaled to fit the screen.
- **Right-to-left** text is laid out with Pango's own line breaking instead of the optimal breaker. In a book whose
  spine says `page-progression-direction="rtl"` the whole reader is mirrored: the left arrow key, a tap on the left
  edge and a swipe to the right turn forward, the footer (arrows, progress bar and labels) runs from right to left
  with the *Next* arrow on the left, and the contents sidebar opens on the right.

## PDF

<figure markdown="1">
![A PDF in the reader](assets/reader-pdf-light.png#only-light)
![A PDF in the reader](assets/reader-pdf-dark.png#only-dark)
<figcaption markdown="1">A PDF opens in the same reader, with the same footer, search and highlights.</figcaption>
</figure>


A PDF opens **as pages**, the way it was made. Next to the PDF the library keeps a reflowed EPUB of its text, which is
what search, the library index and *Convert To* use.

- **Layout**: continuous scrolling or one page at a time (the **View Options** button, the display icon at the right of the header bar). Zoom with Ctrl+plus,
  Ctrl+minus, Ctrl+scroll or a pinch; *Fit width*, *Fit page* and *100 %* are in the same popover, Ctrl+0 fits the width.
- **Crop margins** cuts the white edges (it looks at a sample of pages). **Dark pages** inverts the colours for night
  reading or an e-ink screen with a dark theme.
- **Sidebar** (F9): page thumbnails, the PDF's own contents (bookmarks), your bookmarks and your notes. The footer has a
  slider and a page box that takes a number or a page label (`iv`, `A-3`). Ctrl+G (or Ctrl+L) jumps to it.
- **Links** inside the PDF jump there (the *Back* button returns); links to web addresses open in your browser.
- **Search** (Ctrl+F) lists every match with its page and shows it highlighted on the page.
- **Select text** by dragging (double click: word, triple click: line; selection stays on one page). The popover has the
  same colours, note, copy, dictionary, Wikipedia and translate as in the book reader. Highlights and notes belong to
  the page and are included in *Export Annotations*. Underline, strikethrough and squiggly styles work too.
- **Password-protected** PDFs ask for the password when opened.
- Your **position** and progress are remembered; the library shows the reading progress.
- **Reflow as text (experimental)** in the Main Menu (*Reflow as Text*) opens the reflowed EPUB in the book reader: two columns are read
  one after the other, running headers, footers and page numbers are dropped, hyphenated words are joined. Tables,
  formulas, footnotes and images are not reconstructed. Switch the default in `pdf.json` (`"pages": false`) to open
  PDFs as reflowed text.

!!! note "Limits"
    Scanned PDFs show their pages but have no text to select, search or reflow (no OCR yet). A selection cannot span
    two pages. Vertical and right-to-left PDF text is shown correctly but may select in a different order. Forms,
    comments and other PDF annotations are not drawn or edited. Writing a PDF (EPUB to PDF) is not available.

## Comics and manga

<figure markdown="1">
![A comic in the two-page layout](assets/reader-comic-spread-light.png#only-light)
![A comic in the two-page layout](assets/reader-comic-spread-dark.png#only-dark)
<figcaption markdown="1">The two-page layout; the cover can stay on a page of its own. The pages are from *Little Nemo in Slumberland* by Winsor McCay (1905, public domain).</figcaption>
</figure>


<figure markdown="1">
![A comic page in the reader](assets/reader-comic-page-light.png#only-light)
![A comic page in the reader](assets/reader-comic-page-dark.png#only-dark)
<figcaption markdown="1">A comic page fitted to the window; the toolbar has the comic's own settings. The comic is a Sunday page of *Little Nemo in Slumberland* by Winsor McCay (1905), a public-domain comic from Wikimedia Commons.</figcaption>
</figure>


CBZ and CBT comics (and CBR files that are really zip archives) open in a **comic reader** made for pictures. The
text of ComicInfo.xml fills the library: title, series, number, writer, publisher, year, summary and genre. The same
file says which pages are double pages or covers. Real RAR archives (CBR) are read when the `unrar` or `7z` program is
installed on your computer; without it Bookx says so and you can convert the file to CBZ.

- **Layout** (the **View Options** button, the display icon in the header bar, or the **M** key): *Single page*, *Two pages* (like an open book) or
  *Vertical scroll* for webtoons, where the pages follow each other as one long strip. In the two-page layout
  *Cover alone* keeps the first page alone and *Wide pages alone* gives a double-page picture the
  whole screen; the pairs then continue after it.
- **Fit**: *Fit page*, *Fit width*, *Fit height* or *Original size*. In a page that is taller than the window, Space
  reads down the page and turns to the next one at the bottom.
- **Direction**: a book marked as manga in ComicInfo.xml (`Manga` = `YesAndRightToLeft`) opens **right to left**: the
  Left arrow and a click on the left edge go forward, the first page of a two-page spread is on the right, and the page
  slider runs from right to left. Switch it for a book with *Right to left* or the **R** key; Bookx remembers
  it, the layout and the fit **for that book**. *Use the Defaults for This Book* (it shows only while the book differs
  from the defaults) forgets them.
- **Smart zoom**: double-click (or double-tap) a panel to zoom to it, double-click again to go back. Bookx finds the
  panels by the gutters between them; on a page without gutters it zooms to the region around the pointer.
- **Guided view** (**G** key, or the View Options popover; the menu says only *Guided view*, its tooltip says what it does): Space goes to the next panel, Shift+Space to the one before, across
  pages, in reading order (right to left in manga). The panel is outlined. Without guided view Space reads on a screenful
  at a time.
- **Moving and zooming**: arrow keys turn pages, or move a zoomed page; Page Up and Page Down turn pages; Home and End go
  to the first and last page. Drag to pan, swipe a fitted page to turn it, plus and minus (or Ctrl+scroll, or a
  two-finger pinch) zoom, **0** fits the page again. Clicking near the left or right edge of a fitted page turns it,
  a click in the middle hides or shows the bars.
- **Thumbnails** (F9): the sidebar lists every page as a thumbnail; click one to go there. Pages of a special type in
  ComicInfo.xml (cover, preview, back cover, ...) are named in the tooltip.
- **Grey pages** and **e-ink mode**: pages are shown in grey and moves are not animated. Moves are also not animated
  when your system asks for reduced motion (*Animate moves between panels* in Preferences).
- **Series and volumes**: a comic with no series in ComicInfo.xml gets its series from the folder it was added from
  (`Berserk/Berserk v03.cbz` is volume 3 of *Berserk*; folders such as Downloads or Comics are ignored). On the last
  page, press the forward key once more: Bookx names the next volume of the series, and a second press opens it. The
  same at the first page opens the previous volume at its last page. *View Options → Previous Volume / Next Volume* and
  Ctrl+Page Down / Ctrl+Page Up do it at once. The order is the series number; comics without a series follow the file
  names of their folder (only books that stay where they were, the *Linked* layout).
- **Defaults** for new comics are in *Preferences → Reading → Comics*, including *Read this library right to left* (for
  a library of manga) and *Ignore ComicInfo.xml when importing* (use only file names and picture order).

Pages are decoded in the background and kept ready: the next few pages in your reading direction are prepared while
you read, very large scans are reduced to a size that fits the memory, and the memory used for pages has a ceiling.
Files that claim an absurd picture size (a page of billions of pixels) are not decoded: the page is left out when the
comic is added.

!!! note "Limits"
    Panel detection looks for gutters of one plain colour (white or black) between rectangular panels. Pages with
    overlapping, slanted or borderless panels have no panels found and are shown whole. Bookx does not read the text in
    the pictures, and a screen reader cannot read them.

## Vertical writing

<figure markdown="1">
![Vertical Japanese text, close up](assets/reader-vertical-detail-light.png#only-light)
![Vertical Japanese text, close up](assets/reader-vertical-detail-dark.png#only-dark)
<figcaption markdown="1">Vertical lines run right to left, with the punctuation set for vertical writing.</figcaption>
</figure>


Japanese and Chinese books that are set in vertical lines open the way they were made: the lines run from top to bottom
and the columns follow each other from right to left (`writing-mode: vertical-rl`) or, for some Chinese and Mongolian
books, from left to right (`vertical-lr`). The book decides, there is nothing to switch on.

<figure markdown="1">
![A Japanese book in vertical lines, with furigana](assets/reader-vertical-light.png#only-light)
![A Japanese book in vertical lines, with furigana](assets/reader-vertical-dark.png#only-dark)
<figcaption markdown="1">Natsume Soseki, "Wagahai wa neko de aru" (public domain, Aozora Bunko), in the two-page layout.</figcaption>
</figure>

- **Pages.** A page holds as many columns as fit its width. In a book whose spine says
  `page-progression-direction="rtl"` (all Japanese vertical books should) the first page of a two-page spread is the
  right-hand one, the *Left* arrow key, a tap on the left edge and a swipe to the right turn forward, as for Arabic.
  The single-page layout and the scrolling layout work too; scrolling moves sideways, the mouse wheel and Page Down
  move on in the book.
- **Characters.** Ideographs and kana stand upright, Latin letters and digits are turned a quarter turn
  (`text-orientation: mixed`, the default), or all stand upright (`upright`) or all turn (`sideways`). Punctuation
  uses the vertical forms of the font: brackets turn, `。` and `、` move to the upper right, the long vowel mark
  `ー` stands up.
- **Digits side by side.** Where the book asks for it (`text-combine-upright`, `-epub-text-combine`) two to four digits
  such as `12月24日` are set horizontally in one character cell (*tate-chu-yoko*).
- **Furigana.** `<ruby>` annotations are drawn to the right of their base text in vertical lines and above it in
  horizontal ones. A long annotation spreads its base so that it never runs into the characters beside it. The
  annotation is not part of the text you select, search or read aloud. To hide the furigana, put `rt { display: none }`
  into the custom style sheet in *Text appearance → Advanced and custom CSS*.
- **Line breaking.** Lines are broken by the rules of Japanese typesetting (*kinsoku*): a line never starts with a
  closing bracket, a comma, a full stop, `ー` or a small kana, never ends with an opening bracket, and `――` and `……` are
  not split. Two punctuation marks in a row (`」「`, `。」`) are squeezed to half a character where the font allows it.
- **Selecting and highlighting** work as in horizontal books: drag along a column and on into the next one, search,
  highlight, take notes, read aloud. A double click selects the text between two punctuation marks (Japanese has no
  spaces between words). Highlights and search matches are painted column by column. With
  the keyboard, *Shift+Down* and *Shift+Up* move by character and *Shift+Left* and *Shift+Right* by sentence.
- **Screen readers** get the text in reading order, without the furigana.

A vertical book needs a font with Japanese or Chinese glyphs on your system (for example Noto Serif CJK or Noto Sans CJK,
IPAex Mincho, Source Han). Without one the characters show as empty boxes. If you turn *Publisher styles* off, a vertical
book stays vertical (the direction of the lines is not a matter of style), but loses its indents and its digit
combining.

!!! note "Limitations"
    - Vertical text: punctuation that hangs outside the line end (*burasage*) and `line-break`, `word-break` and
      `text-emphasis` (emphasis dots) are not supported; the justified lines are spread evenly instead. A chapter is laid
      out in one writing mode (the one of its `html` or `body`); a horizontal block inside a vertical chapter is laid out
      vertically too. Floated pictures do not float in vertical text, they stand in the flow. Code backgrounds and quote bars
      are drawn for horizontal text only.
    - Fixed layout: no two-page spreads and no absolutely positioned text.
    - The reader draws its own text, so the book's CSS is applied only as far as listed under *the book's own
      styling*. The page text is exposed to screen readers (see [Accessibility](accessibility.md)), but the words inside pictures and the alternative text of images are not read.
    - [Read aloud](read-aloud.md) works on Linux only; natural voices need `espeak-ng` and a command-line audio player.
    - Touch gestures and e-ink mode have been tested with simulated input, not on a real touch or e-ink screen.
    - There is no cloud sync of reading progress; the only sync is [Bookx sync](progress-sync.md),
      which you switch on yourself.

## Questions and answers

??? question "How do I make a dictionary answer first?"
    Preferences → **Dictionaries** → *Dictionary order*: **Move Up** on the dictionary you want first. The order is
    remembered; a dictionary you add later is asked last.

??? question "How do I look a word up on a web site I choose?"
    Preferences → Dictionaries → *Web look-up sites* → **Add Site**, with an address such as
    `https://dictionary.example.com/search?q={word}`. In the look-up popover the **Web** menu then lists it.

??? question "Where is the list of words I looked up, and how do I clear it?"
    Preferences → Dictionaries → *Look-up history* → **Show**, or **Web → History** in the look-up popover. **Clear**
    empties it and a message offers **Undo**; switch *Keep a history of look-ups* off to stop recording.

??? question "How do I search for a whole word or with a pattern?"
    Open the search (++ctrl+f++) and use the buttons under the field: **Words** for whole words, **Aa** to match case and
    **.\*** for a regular expression.

??? question "How do I make the page warmer in the evening?"
    **Preferences → Reading → Reading aids → Evening schedule**: switch it on and set the two times and the warmth.

??? question "How do I change the font, the size or the line spacing?"
    Click the **Text Appearance** button in the reader's header bar (the text-format icon, next to the magnifier). The
    popover has the **Font** list, the size buttons (smaller, bigger, back to 100 %) and a **Line spacing** slider with its
    value next to it. The change applies at once and is kept for this book only. ++ctrl+plus++, ++ctrl+minus++ and
    ++ctrl+0++ change the size from the keyboard.

??? question "How do I change the margins or the number of columns?"
    Open **Text Appearance** in the header bar: **Margins** is a slider, and **Columns** chooses two columns, a single
    column or continuous scroll. If the window is narrow the button moves into the Main Menu (the hamburger at the far right).

??? question "How do I switch to a dark or sepia theme?"
    Open **Text Appearance**: the round swatches at the top are *follow the system*, *light*, *sepia*, *dark* and *black*.
    Pressing ++t++ with nothing selected cycles through them. The **+** swatch makes your own theme.

??? question "How do I use the colours the book itself asks for?"
    Open **Text Appearance**, then **More Text Options**. Under **Colours** choose *Original* (the book's own colours) or
    *Custom* (your own text and page colours). *Theme* goes back to the theme you picked.

??? question "How do I turn on hyphenation?"
    Open **Text Appearance**, then **More Text Options**, and switch on **Hyphenate words** in the *Text* group; the same
    window has *Paragraph spacing* and *Optimal line breaking*. To switch hyphenation on for every book, use
    **Preferences → Reading → Hyphenate words**. It uses the language of the book and works best with **Justify text**.

??? question "How do I use my own CSS or make my look the default for new books?"
    **More Text Options** has an *Advanced and custom CSS* section (collapsed) for your own style sheet, and a *This book*
    section: **Use as Default for New Books** copies the current look to books you have not changed, **Reset This Book to
    Defaults** forgets this book's look.

??? question "How do I highlight text?"
    Select a word or a passage (double-click, drag, long-press on a touch screen, or ++shift+right++ on the keyboard).
    A small popover appears: click one of the five colours. ++h++ highlights the selection from the keyboard.

??? question "How do I add a note to a passage?"
    Select the text and click the pencil (*Add Note*) in the popover, or press ++n++. Type the note and click away or press
    ++ctrl+enter++ to save it. Click the highlight later to edit the note.

??? question "How do I change a highlight's colour or style?"
    Click the highlight on the page: the same popover opens, with the five colours and the four styles (background,
    underline, strikethrough, squiggly). Pick another one. In the sidebar, the **⋮** button of a note offers the same colours
    and styles.

??? question "How do I remove a highlight?"
    Click the highlight and press the bin (*Remove Highlight*) in the popover, or use the bin in the sidebar's **Notes** list.
    The note goes with it. A message with **Undo** stays on screen for ten seconds.

??? question "How do I see all my highlights and notes?"
    Press ++f9++ (or the sidebar button at the left of the header bar) and open the **Notes** tab. You can search them and
    filter them by colour, by style and by notes only. To save them as a file, see [Export Annotations](annotations.md).

??? question "How do I look up a word in the dictionary?"
    Double-click the word, or select a phrase, and click the dictionary button in the popover (or press ++d++). Bookx uses
    offline dictionaries; if there is none for the language of the book, the popover offers a download. Dictionaries are
    managed in **Preferences → Dictionaries**.

??? question "How do I look something up on Wikipedia or translate it?"
    Select the text and click the Wikipedia or Translate button in the popover, or press ++w++ or ++t++. Both need a network
    connection. Translate has a language picker.

??? question "How do I save a word to my vocabulary?"
    Looked-up words are added automatically. To add a term of your own with your own definition, select it and click the **+** button
    of the popover (or press ++v++). The **Vocabulary** window is in the **Library** menu of the library's header bar.

??? question "How do I bookmark a page and go back to it?"
    Click the bookmark button at the left of the header bar, or press ++ctrl+d++. Press it again to remove the bookmark. Open
    the sidebar (++f9++) and the **Bookmarks** tab to see them all; click one to go there.

??? question "How do I search in the book?"
    Click the magnifier in the header bar, or press ++ctrl+f++, and type. Every match is listed with its chapter; ++enter++ or
    a click jumps to it and highlights it on the page.

??? question "How do I jump to a page, a percentage or a chapter?"
    Open the Main Menu and choose **Go to** (or press ++ctrl+l++ or ++ctrl+g++). Type `45` for a page, `37%` for a percentage,
    or part of a chapter name. The line under the field says where it will go. You can also drag the slider under the page,
    or use the contents in the sidebar.

??? question "How do I go back to where I was after a jump?"
    After a link, a search result or *Go to*, a **Previous Position** button appears at the left of the header bar. Click it, or
    press ++alt+left++; ++alt+right++ goes forward again.

??? question "How do I see where I am in the whole book?"
    Open the Main Menu and choose **Book Map** (or press ++m++): every page as a small block, with the pages you have read,
    your highlights and your bookmarks. Click a block to go there.

??? question "How do I change what the footer shows?"
    Right-click the footer, or open the Main Menu and choose **Footer Statistics**. Choose a preset, switch items on and off
    and move them up or down. **Only for this book** gives the open book a footer of its own.

??? question "How do I use the reading ruler?"
    Open the Main Menu and tick **Reading Ruler**. The page is dimmed except for a band that follows the pointer (or your finger);
    ++alt+up++ and ++alt+down++ move it line by line. The number of lines and the dimming are in **Preferences → Reading**.

??? question "How do I use speed reading?"
    Open the Main Menu and choose **Speed Reading**. A small card with play / pause and faster / slower appears over the page;
    ++space++ starts and pauses, ++up++ and ++down++ change the speed, ++escape++ closes it.

??? question "How do I make Bookx read the book aloud?"
    Click the speaker button in the header bar and press play (or press ++r++). It reads from your selection or the top of the
    page and highlights the sentence. See [Read aloud](read-aloud.md).

??? question "How do I change the voice or the speed of read aloud?"
    On the player card that floats over the page, open the full player with the arrow button: **Slower** and **Faster** set the
    speed, and the voice button chooses a voice for the language of the book. *Voice Settings* (in the speaker popover or the
    voice list) opens **Preferences → Text to Speech** with pitch and engine.

??? question "How do I set a sleep timer?"
    Open the full player (the arrow button on the card) and click the alarm-clock button: stop after 15, 30, 45 or 60 minutes,
    or at the end of the chapter.

??? question "How do I read a PDF?"
    Open it from the library like any book. It opens as pages. The **View Options** button (display icon) in the header bar
    sets continuous or single-page layout, zoom, crop margins and dark pages; *Reflow as Text* in the Main Menu opens the
    reflowed text instead. See [PDF](#pdf).

??? question "How do I read a comic or manga?"
    Open the CBZ or CBT file from the library. The **View Options** button in the header bar chooses single page, two pages or
    vertical scroll, the fit and the reading direction; double-click a panel to zoom to it, ++g++ starts guided view. See
    [Comics and manga](#comics-and-manga).

??? question "How do I listen to an audiobook?"
    Audiobooks are added with **Add Audiobook** in the **+** menu of the library and open in their own player. See
    [Audiobooks](audiobooks.md).

??? question "How do I keep my reading position in step with KOReader?"
    Switch on Bookx sync in **Preferences → Sync**; on the e-reader use KOReader's progress sync or the Bookx plugin. The steps
    are in [Bookx sync](progress-sync.md) and [KOReader highlights and notes](koreader-sync.md).

??? question "How do I use e-ink mode or a touch screen?"
    Switch on **E-ink mode** in **Preferences → Reading**: black on white, no animations, bars that stay and wide tap zones.
    The same page has **Tap zones** for touch screens. There is nothing to switch on in the reader itself.

??? question "How do I read with the keyboard only?"
    ++right++ / ++left++ turn pages, ++ctrl+right++ / ++ctrl+left++ change chapter, ++ctrl+f++ searches, ++ctrl+l++ goes to a page,
    ++f9++ opens the sidebar, ++f10++ the Main Menu, ++ctrl+k++ the command palette. You can even select, look up and highlight
    with the keyboard: see [Accessibility](accessibility.md#reading-and-selecting-with-the-keyboard-only) and
    [Keyboard shortcuts](shortcuts.md).

??? question "How do I make the header and bars go away?"
    They fade by themselves after a few idle seconds. Press ++f11++ (or double-click the margin) for fullscreen. In e-ink mode
    the bars stay.

??? question "What if a book will not open?"
    The page says why and has a **Back to Library** button. Damaged or missing files are the usual reason; check the book with
    *Library Health* in the library's main menu.
