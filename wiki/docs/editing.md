# Editing

The editor is for fixing and polishing EPUB files. To open it, right-click a book in the library and choose
**Edit → Edit Book** (the entry is missing when several books are selected). Each book opens in its own window. A
book of another format can be opened too: see [Book menu](#book-menu) below.

<figure markdown="1">
![The book editor](assets/editor-light.png#only-light)
![The book editor](assets/editor-dark.png#only-dark)
<figcaption markdown="1">Real screenshot.</figcaption>
</figure>

## Overview

The window has a Files list on the left, a code editor in the middle and a **live preview** on the right. The
preview is drawn by the same engine as the reader, so what you see is what the reader shows.

The **header bar** at the top holds the buttons you use most:

- at the start: **Save** (++ctrl+s++, writes the book back into the library), **Undo** and **Redo** (these two
  undo and redo typing; undoing a whole tool is a different thing, see [Compare and checkpoints](#compare-and-checkpoints));
- at the end, left to right: **Find** (++ctrl+f++), **Preview** (shows or hides the preview), **Tools**, **Markup**
  and the **main menu** (☰, ++f10++), which this page calls the *Book menu*.

The **sidebar** on the left has four tabs, shown as icons (hover for the name): *Files*, *Contents*, *Metadata*
and *Live CSS*. The **Problems** panel, the search results and the spelling panel open below or beside the code when
you ask for them, and a status line at the bottom shows the cursor position.

- The code editor handles XHTML, CSS, OPF and NCX with **syntax highlighting, line numbers and undo/redo**. To
  tidy XML, open the Tools menu in the header bar and choose Fix & Clean → *Reformat Markup*. See
  [The code view](#the-code-view) below.
- Press ++ctrl+shift+p++ to open the **command palette** and type what you want to do. It finds editor commands,
  every item of the Tools, Markup and Files menus, and the files of the book (++ctrl+k++ opens the palette in the
  library and reader, but in the editor it inserts a hyperlink).
- *Metadata* and *Cover* editing is in the editor too: the *Metadata* tab of the sidebar, and any image of the book
  can be used as the cover.

## The code view

The code view is built on GtkSourceView, the editing widget of GNOME's own editors.

<figure markdown="1">
![The code view with the minimap, the marks column and a misspelled word](assets/editor-light.png#only-light)
![The code view with the minimap, the marks column and a misspelled word](assets/editor-dark.png#only-dark)
<figcaption markdown="1">Real screenshot.</figcaption>
</figure>

- **Highlighting** for XHTML, XML, OPF, NCX, CSS (and SVG, JavaScript and Markdown files). The colours follow the app:
  light, dark, and a black-on-white scheme in e-ink mode.
- Line numbers, a highlight on the current line, **bracket matching**, automatic indent, Tab to indent, and *Home* /
  *End* that go to the first character of the line first. Tab inserts two spaces.
- **Tag pair highlight:** with the caret inside a tag, that tag and its partner (`<p>` … `</p>`) are highlighted.
- **Minimap** at the right edge shows the whole file; click it to jump. Switch it off in the header bar's Markup
  menu → View → *Show Minimap*. The choice is remembered. (In a window narrower than about 1000 px it is hidden anyway.)
- **Show Hidden Characters** (Markup menu → View) draws spaces, tabs and line breaks, and marks no-break spaces,
  zero-width characters, soft hyphens and other characters that are easy to miss in yellow. When the caret is on one,
  the status line names it.

### Marks column

<figure markdown="1">
![The code view with line numbers and squiggles under misspelled words](assets/editor-code-light.png#only-light)
![The code view with line numbers and squiggles under misspelled words](assets/editor-code-dark.png#only-dark)
<figcaption markdown="1">The code view: line numbers, syntax colours, and red squiggles with a count in the marks column for possible misspellings.</figcaption>
</figure>


The narrow column left of the line numbers shows, per line:

| Mark | Meaning |
|---|---|
| Green bar | the line is new since the last [checkpoint](#compare-and-checkpoints) (or since you opened the editor when there is none) |
| Blue bar | the line was changed since then |
| Red tick | lines were deleted just above |
| Red, amber or blue icon | a finding of the last book check (++f7++) is on this line |
| Red number | how many words on the line are possibly misspelled |

Hover a mark for the text of the finding. Findings follow the text as you edit, but they are not re-checked until you
run the check again (++f7++, or Tools → *Check Book*).

### Spelling in the text

<figure markdown="1">
![The Spelling panel](assets/editor-spelling-light.png#only-light)
![The Spelling panel](assets/editor-spelling-dark.png#only-dark)
<figcaption markdown="1">The Spelling panel: every possibly misspelled word with its count, then *Change All*, *Ignore*, *Ignore in Book* and *Add to Dictionary*.</figcaption>
</figure>


Possibly misspelled words in XHTML files are underlined in red while you type, using the same Hunspell dictionary as
Tools → *Check Spelling* (the book's language; none is bundled). Right-click a word for **Replace with**,
**Ignore** (until the editor closes) and **Add to Dictionary** (kept in `spelling-words.txt` next to the settings).
Files bigger than about 700 kB are not checked while typing. If you do not want the underlines, switch off
*Underline misspelled words* in Preferences → Editor (see [Editor preferences](#editor-preferences)). The whole-book
spelling panel is described under [Spelling](#spelling).

### Completion

<figure markdown="1">
![The completion list after typing an angle bracket and two letters](assets/editor-completion-light.png#only-light)
![The completion list after typing an angle bracket and two letters](assets/editor-completion-dark.png#only-dark)
<figcaption markdown="1">Completion after `<di`: tag names with their kind and a *Details* button.</figcaption>
</figure>


Completion pops up while you type, or press ++ctrl+space++.

- after `<`: tag names; inside a tag: attribute names for that tag (picking one adds `=""` and puts the caret inside);
- in `href`, `src`, `poster` and CSS `url()`: the files of the book, with the relative path worked out for you. The
  letters you type only have to appear in order, and word starts count most: `c2` finds `chapter-2.xhtml`, `s.css`
  finds `style.css`. After `#` (or `file.xhtml#`): the ids in that file (including ones you typed and have not
  saved), each with the first words of the text it points at. Press ++tab++ or ++enter++ to take the highlighted one;
- in `class="…"`: the classes your stylesheets define; in a CSS rule: property names;
- in text: words from the open file and snippets whose trigger matches.

### Snippets

<figure markdown="1">
![The Manage Snippets dialog](assets/editor-snippets-light.png#only-light)
![The Manage Snippets dialog](assets/editor-snippets-dark.png#only-dark)
<figcaption markdown="1">Manage Snippets: your own triggers, the files they apply to and a test box.</figcaption>
</figure>


A snippet is a short word that expands into text with tab stops. Type the word and press ++tab++. Tab moves to the next
stop, `${1:text}` stops hold a default, and the caret ends where `$0` is. Built in: `p` (paragraph), `h` (heading),
`img`, `link`, `sec` (a section with `epub:type`), `fn` (footnote reference), `aside` (footnote), `Lorem` (three filler
paragraphs), `<>` (self-closing tag), `<a` (link), `<i` (image), `<<` (any tag) and `<c` (tag with a class) and, in
stylesheets, `rule`. Triggers may start with `<`: type `<a` and press ++tab++.

**Wrapping the selection.** A stop written `${2*}` (or `${2*:default}`) takes the text that was selected when you expand
the snippet: select a word, type nothing, and press ++ctrl+j++ (**Insert Snippet**, a searchable list), or type the
trigger just before the selection and press ++tab++. `p`, `h`, `link`, `<<`, `<c` and `<a` use it. Without a selection
the stop shows its default.

**In the search fields.** Tab in *Find* or *Replace* expands a trigger there too, as plain text: stops show their
default and nothing jumps.

The Markup menu in the header bar → Insert → **Manage Snippets** adds your own: name, trigger, files it applies to and text. The **Test** box expands the
snippet in a scratch box (with sample selected text), so you see what you get before saving. The buttons in the title bar
import and export your snippets as a JSON file to share them. A snippet of yours with the same trigger replaces a
built-in one. They are stored as `snippets.json` next to the settings. The same dialog lists your **templates for new
files** (XHTML or CSS; `{title}` is replaced by the name), the ones **New File** offers (see [Files and tabs](#files-and-tabs)).

### Editor preferences

<figure markdown="1">
![The Editor Preferences dialog](assets/editor-preferences-light.png#only-light)
![The Editor Preferences dialog](assets/editor-preferences-dark.png#only-dark)
<figcaption markdown="1">Editor Preferences: font, tabs, line numbers, margin guide, wrapping and the overview map.</figcaption>
</figure>


To change how the code looks and behaves, open the Book menu (☰ at the end of the header bar) and choose
**Preferences** (++ctrl+comma++), then the **Editor** page. The command palette's *Editor Preferences* entry opens
the same settings in a smaller window. Every open editor window follows a change at once.

- **Code**: font and size, tab width, spaces or tabs, line numbers, highlight of the current line, a right margin guide
  at a column, wrapping, showing spaces and tabs, the overview map.
- **Colour scheme**: a list of the available schemes with a preview of each, or *Follow light and dark mode*. Your own
  GtkSourceView schemes (`*.xml`) go into the `styles` folder of the Bookx configuration (the page has a button that
  opens it) and appear in the list.
- **Editing**: close tags automatically (typing `>` after `<p` adds `</p>`), suggest completions while typing (or only
  on ++ctrl+space++), paint colour codes such as `#1a2b3c` in style sheets in that colour, underline misspelled words,
  turn typed entities into characters.
- **Preview**: font, smallest text size, whether the preview follows the cursor, whether scrolling the preview scrolls
  the code.

The cursor's width and blinking follow your desktop settings. Keyboard shortcuts of the editor are under
**Preferences → Keyboard Shortcuts**, in the *Editor* group. *Reopen tabs and cursors* (the open tabs and the caret
of each file, remembered per book) is the last switch of the *Editing* group, and *Wrap Long Lines* is also in the
Book menu.

### Files with one very long line

Some EPUBs are written without line breaks, so a whole chapter is a single line of hundreds of kilobytes. A text view
cannot show such a line without freezing for seconds, so Bookx does not put the file into the code view. The code
view shows a page that says so, with two choices: **Add Line Breaks and Open** puts the tags on separate lines (only the
white space between tags changes; *Undo* takes it back) and **Open Anyway** shows the file as it is, which can make the
editor stop responding for a few seconds. The preview of the chapter is not affected. Files of more than about 50 KB
on one line are never coloured or underlined while they stay in one line; after line breaks are added they are
coloured again.

## Files and tabs

<figure markdown="1">
![The Metadata tab of the editor](assets/editor-metadata-tab-light.png#only-light)
![The Metadata tab of the editor](assets/editor-metadata-tab-dark.png#only-dark)
<figcaption markdown="1">The *Metadata* tab: cover, title, authors and tags of the book you are editing.</figcaption>
</figure>


The **Files** tab (first icon of the sidebar) lists every file of the book, grouped by type, with reading-order
numbers. The row of small buttons under the list adds files and moves a page up or down in the reading order; the
`⋯` button next to them opens the **Files menu**, which is also what you get when you right-click a file. Most of what
follows is in that menu.

- Select several files with Ctrl/Shift-click.
- **Rename** with ++f2++ (or Rename in the Files menu). Every `href`, `src`, CSS `url()` and `#fragment` that points at the file is repaired; other
  files with similar names are left alone.
- **Delete** with ++delete++ (or Delete Selected in the Files menu). One file that nothing else refers to goes at once and a toast offers **Undo** for about
  ten seconds. Two or more files, or a file a page, stylesheet or the contents still points to, ask first and say how
  many. The *Images* dialog does the same; its button says how many images are selected ("Delete 3 Images").
  Reorder the spine by moving files up or down.
- In the Files menu (the `⋯` button or a right-click): *New File* (templates, optional stylesheet links, ++ctrl+n++),
  *Add Files* (any file from your computer: a picture, font, stylesheet or audio file; Bookx puts it in the folder
  that suits its type and lists it in the book's package), *Merge Selected*, *Bulk Rename*, *Link Stylesheets* and
  *Export Selected*. [File list extras](#file-list-extras) below describes the rest of the menu.
- Files you open get **tabs**. ++ctrl+w++ closes one, ++ctrl+alt+down++ / ++ctrl+alt+up++ step through the reading
  order.
- **Quick open** with ++ctrl+p++: type a few letters of a file name and press ++enter++.
- The **command palette** (++ctrl+shift+p++, or *Command Palette* with your own keys under Preferences → Keyboard
  Shortcuts) lists the editor's commands (save, find, check, go to line …), every
  item of the Tools, Markup and Files menus and the files of the book. Type a few letters and press ++enter++.

![The editor's command palette](assets/editor-palette-light.png#only-light)
![The editor's command palette](assets/editor-palette-dark.png#only-dark)

Every change that touches the whole book first takes a checkpoint named "Before: …". The toast that reports the
change (a tool, Replace All in the whole book, a restore) has an **Undo** button for about ten seconds: it puts the
book back and removes that checkpoint again. After the toast is gone, open the Book menu (☰) and choose *Revert to
Before the Last Change* (++ctrl+alt+z++); it shows a **Redo** button, and *Revert to After (Redo)* an **Undo**
button, the same way.

## Search and replace

Press ++ctrl+f++ (or the **Find** button at the end of the header bar) to open the find bar, one row high above the
code. The **Search Options** button at its end holds the mode, match case, whole words, scope and the other options;
++ctrl+h++ adds the replace row. Inside that popover the `⋯` button has *Saved Searches*, *Save This Search* and
*Functions*.

<figure markdown="1">
![Find and replace in the editor](assets/editor-find-light.png#only-light)
![Find and replace in the editor](assets/editor-find-dark.png#only-dark)
<figcaption markdown="1">Real screenshot.</figcaption>
</figure>

- Modes: text, regular expression (look-around, `\1` back-references), **function mode** (a script decides each
  replacement), and **tag search** (elements and attributes). Searching **ignoring HTML markup** finds a phrase that is
  split by `<i>` or `<b>`.
- Scopes: this file, text files, style files, all files, selected files, open files, or marked text.
- **All matches in the open file are highlighted** in the text as you type, with a live "n of m in this file" count
  (also in the minimap). *Find All* lists the matches of every file in the scope, *Count* counts them all.
- Saved searches, search history, and the full description of every mode: see [Search and replace](search-and-replace.md).
- *Go to line* (++ctrl+l++) and *Mark selected text* (++ctrl+shift+m++).

### Jumping to definitions

Hold ++ctrl++ and click (or press ++f12++, *Go to Definition*, with the caret there):

- a **file name** in `href`, `src`, `poster`, CSS `url()` or `@import` opens that file; with `#anchor` the caret goes to
  the element that has the id;
- a **class name** in a page goes to its first rule in the style sheets (the ones the page links come first) and selects
  it; a **class selector** in a style sheet goes to the first page that uses the class. The same works for `id` and `#id`.

A web address only shows a message; nothing is opened from the editor. Right-click a class name for **Rename Class**:
type the new name, look at the list of files and changed lines, and press *Rename*. Bookx takes a checkpoint first, then
renames every use in the pages, the style sheets and `<style>` blocks (whole class names only); each file keeps its own
undo history. Right-click a tag, an attribute or a CSS property for **Help for …**, which opens the MDN page in your
browser.

### Underlines and colours

<figure markdown="1">
![A CSS rule with colour swatches behind the colour codes](assets/editor-swatches-light.png#only-light)
![A CSS rule with colour swatches behind the colour codes](assets/editor-swatches-dark.png#only-dark)
<figcaption markdown="1">Colour codes in a stylesheet are painted in their own colour.</figcaption>
</figure>


These are drawn while you type and can be switched off in the **View** part of the Markup menu (the `<>` button in
the header bar):

- **Invalid markup** has a red squiggle (unclosed or stray tags, duplicate or unquoted attributes, a bare `&`, unbalanced
  braces in CSS); hover for the reason. Tools → *Check Book* does the full validation.
- **Link targets** are underlined; a link to a file or anchor that does not exist is underlined in red, with a tooltip.
- **Colour swatches** sit behind `#rgb`, `rgb()`, `hsl()` and colour names in style sheets, `<style>` blocks and `style`
  attributes, with black or white text, whichever is easier to read.
- **Draw Bold and Italic Text** (off by default) draws the text inside `<b>`, `<i>`, `<em>`, `<strong>` and headings
  that way.

Files over about 1.5 MB are not decorated. **Close Tags When Typing `</`** (on by default) completes the name of the
open element when you type `</`; typing the rest of the closing tag yourself works as usual.

## Tag actions

<figure markdown="1">
![The Markup menu](assets/editor-markup-menu-light.png#only-light)
![The Markup menu](assets/editor-markup-menu-dark.png#only-dark)
<figcaption markdown="1">The Markup menu: Format, Insert, Structure, split at the caret, go to definition and sort CSS rules.</figcaption>
</figure>


The `<>` button in the header bar opens the **Markup menu**. It has short pages so it never needs scrolling: **Format** (text styles,
colour, alignment, case, block type), **Insert** (hyperlink, image, special character, snippets), **Structure** (matching
tags, wrap, rename, comment, and the split entries), three commands (*Split Tag at Caret*, *Go to Definition*, *Sort CSS
Rules*) and **View** (the switches of the editor). Where a command has a key, the menu shows it next to the item; the label
follows the key when you change it in Preferences → Keyboard Shortcuts.

| Action | Shortcut |
|---|---|
| Select to the matching tag | ++ctrl+alt+k++ |
| Jump to the matching tag | ++ctrl+alt+j++ |
| Close the open tag | ++ctrl+alt+period++ |
| Wrap the selection in a tag | ++ctrl+alt+w++ or ++ctrl+less++ |
| Remove the surrounding tag | ++ctrl+alt+u++ or ++ctrl+greater++ |
| Split the tag at the caret (a paragraph is cut in two there) | ++ctrl+alt+greater++ |
| Bold / italic / underline | ++ctrl+b++ / ++ctrl+i++ / ++ctrl+u++ |
| Strikethrough, superscript, subscript, code | Format submenu; set keys in Preferences |
| Insert hyperlink / insert image | ++ctrl+k++ / ++ctrl+alt+m++ |
| Go to definition | ++f12++ or ++ctrl++ + click |
| Text colour / background colour | Format → Colour; set keys in Preferences |
| Align left / centre / right / justify | Format → Alignment; set keys in Preferences |
| Sort CSS rules | menu only |
| Rename a tag | ++ctrl+alt+r++ |
| Toggle a comment | ++ctrl+slash++ |
| Change a block to a paragraph / heading 1-6 | ++ctrl+0++ … ++ctrl+6++ |
| Change case | menu only |

The format actions put the right tag around the selection and take it off again when the selection is exactly that
element (or inside it: the element is closed and reopened around your selection). A selection that cuts through tags is
refused with a hint. **Insert Hyperlink** and **Insert Image** open a dialog: type a few letters of a file name to filter
(abbreviations work), pick an anchor inside a page, set the link text, title or alt text, and read the markup that will be
inserted before you press *Insert* (++enter++). With the caret inside a link, the dialog changes that link's address
instead of adding a second one. **Sort CSS Rules** orders the plain rules by selector, also the rules inside `@media`,
`@supports`, `@layer` and `@container` blocks (each block on its own); other at-rules and comments stay in their place.

**Text colour** and **background colour** (Format → Colour) open the system colour chooser and wrap the selection in
`<span style="color:#rrggbb">` (or `background-color`). Choosing the same colour again takes the span off, another colour
replaces it, and *Remove Text Colour* / *Remove Background Colour* clear it. **Alignment** (Format → Alignment) writes
`style="text-align:…"` on the paragraph, heading or other block around the caret; the same alignment again, or *Remove
Alignment*, takes it off, and other style declarations of the block stay. Switch on **View → Use CSS Classes for Colours
and Alignment** to get `class="color-ff0000"`, `class="bg-ffff00"` or `class="align-center"` instead; the rule is added to
the book's first style sheet (and shown in the editor, so you can undo it there).

Hold ++ctrl++ over a link target, a class name or an id and it is underlined and the pointer turns into a hand: a click
there jumps to the target. Typing `</` closes the open tag also when the `/` comes from an input method (a compose key
or an on-screen keyboard).

## Checking the book

<figure markdown="1">
![The problems panel after a book check](assets/editor-problems-light.png#only-light)
![The problems panel after a book check](assets/editor-problems-dark.png#only-dark)
<figcaption markdown="1">F7 lists the findings with their file and line; a click jumps there, and the panel explains the selected one.</figcaption>
</figure>


Press ++f7++, or open the Tools menu in the header bar and choose **Check Book**. The **Problems** panel opens at
the bottom of the editor: a list grouped by file, with a red, amber or blue icon per problem, the line and column, the check id (such as `OPF-003`) and count
badges for errors, warnings and notes.

- **Click** a problem to read what it means, why it matters and how to fix it, in plain language.
- **Double-click** it, or press ++enter++, to open the place in the editor.
- ++f8++ and ++shift+f8++ go to the next and the previous problem (the arrow buttons in the panel's header do the same).
- **Fix** repairs the problem in its file; **Fix all of this kind** repairs every problem of that kind in the book.
  Both take a checkpoint first ("Before: Fix …"), so Tools → Checkpoints → *Restore Checkpoint* undoes them (or press **Undo** in the toast), and the list is
  checked again afterwards. Problems that cannot be repaired automatically show no Fix button.
- **Ignore this kind** hides every problem of that kind in this book. The options menu (`⋯`) at the right of the
  panel's header lists the ignored kinds and shows them again. Problems with the markup or the package itself cannot be ignored.
- **Also check accessibility** (a switch in the same options menu) adds hints such as skipped heading levels,
  missing roles, missing accessibility metadata and SVG pictures without a title.
- After you edit a file, its entries are dimmed until you check again. Checking again keeps the selected problem selected.

What each Fix does:

| Problem | Fix |
|---|---|
| File not in the manifest (`OPF-003`) | adds it to the manifest |
| Manifest entry without a file (`RSC-001`) | removes the entry |
| Markup not well-formed (`RSC-016`), no namespace (`HTM-901`) | repairs the markup / adds the namespace |
| Repeated ids (`HTM-905`, `OPF-906`) | gives them new names |
| Image without `alt` (`HTM-907`) | marks it as decorative |
| Broken link that differs only in case or folder (`RSC-007`, `BKX-022`) | rewrites the link |
| Unsafe file name (`PKG-009`, `PKG-010`) | renames the file; links follow |
| Empty file, unused file, leftover bookmarks file (`BKX-001`, `BKX-024`, `BKX-026`) | deletes the file |
| Text not in UTF-8 (`BKX-002`, `BKX-003`) | converts the file to UTF-8 |
| Named or broken character references (`BKX-004`, `BKX-005`, `BKX-006`) | numeric references, `&amp;`, or the entity's text |
| Invalid id (`BKX-008`) | renames the id; links to it follow |
| Text directly in `<body>` (`BKX-009`) | wraps it in `<p>` |
| CMYK JPEG (`BKX-016`) | converts it to RGB |
| Link with a space or backslash (`BKX-019`) | writes the character in its safe form |
| Page not in the reading order (`BKX-025`) | adds it at the end |
| More than one cover (`BKX-028`) | keeps the first |
| Missing metadata, identifier or version (`OPF-9xx`, `OPF-048`, `OPF-030`, `OPF-001`) | adds or repairs it |
| Missing ARIA roles or accessibility metadata (`ACC-902`, `ACC-903`, `ACC-904`) | adds them |

The check combines the quick checks of the editor with an EPUB validator and extra checks for things e-readers
trip over: empty files, encodings, entities, ids, CSS declarations, fonts that cannot be read or may not be embedded,
damaged or CMYK images, links to folders or with wrong case, unused files and more.

## Table of contents

<figure markdown="1">
![The Contents tab of the editor](assets/editor-contents-tab-light.png#only-light)
![The Contents tab of the editor](assets/editor-contents-tab-dark.png#only-dark)
<figcaption markdown="1">The *Contents* tab: the table of contents as a tree; drag entries to move or re-nest them, then *Generate*.</figcaption>
</figure>


The **Contents** tab (second icon of the sidebar) is the contents editor. It renames (type in the entry's title),
indents, reorders and links entries, and writes both `toc.ncx` and `nav.xhtml`. Its toolbar at the bottom of the
sidebar has *Add*, *Delete*, a *Move entry* menu (up, down, in, out), the pointer button described below and *Generate*.
Selecting an entry opens its file at the anchor. *Generate* builds a contents from the headings (levels 1-6), from
the links of a contents page, or one entry per file, replacing the current one or adding to it.

The list only builds the rows in view, so a book with tens of thousands of entries still opens at once and scrolls
smoothly.

**Moving entries.** Drag the grip at the left of an entry onto another entry. Where you let go on the row decides:
the top quarter puts the entry *before* it, the bottom quarter *after* it (and after its sub-entries), the middle
makes it the *last sub-entry*. An entry always moves with its sub-entries, and nothing can be dropped into itself.
The same moves work from the keyboard: ++alt+up++ / ++alt+down++ move the selected entry among its siblings,
++alt+right++ makes it a sub-entry of the entry above, ++alt+left++ moves it out, after its parent. *Delete* keeps
the sub-entries of the deleted entry (they move up a level) and shows an **Undo** toast. *Add* puts a new entry after the selected one; when the
caret of the open page is in or after an element with an `id`, the entry points at that element and takes its
heading text as the title. Everything is written to `nav.xhtml` and `toc.ncx` when you save.

To point an entry at an exact place, select it and press the *Point the entry at a place you click in the preview*
button in the Contents toolbar, then click where it should lead in the preview. A green line shows the place under
the pointer. Bookx adds an `id` to the element if it has none and sets the entry to `file#id`.

## Spelling

While you type, misspelled words are underlined (see [The code view](#spelling-in-the-text)). **Tools → Check Spelling**
opens a panel at the right of the editor with every misspelled word of the whole book, its language and how often it
occurs. Bookx uses the Hunspell dictionaries installed on your system (for example from `hunspell-en`), plus the ones
you import. None are bundled.

- **Select a word** to see its next occurrence selected in the editor, with suggestions below the list. Press ++enter++
  (or double-click) to jump there and put the focus in the editor.
- **Change All** replaces every occurrence in the book with the suggestion you picked or the word you typed. A checkpoint
  is taken first, so you can go back.
- **Ignore** hides the word until the editor closes; **Ignore in Book** remembers it for this book. Both, and **Add to Dictionary**, show a toast with **Undo**.
- **Add to Dictionary** puts the word into one of your dictionaries (*Personal* or one you made). Words are plain text files
  in the settings folder (`spelling-words.txt` and `spelling-dictionaries/`).
- The toggle with the list icon shows **all words** with their counts, not only the misspelled ones; the filter field
  narrows the list. **Refresh** checks again after you edited the book.
- Text marked with `lang` or `xml:lang` is checked with the dictionary of that language, so `en-GB` and `en-US` text each
  get their own. A language without a dictionary is left unchecked and named in the panel.
- ++ctrl+f8++ / ++ctrl+shift+f8++ jump to the next / previous misspelled word, continuing in the next file at the end of a file.
- A file can be left out: menu **Exclude This File from Spell Check**, or put the comment `<!-- bookx-no-spell-check -->`
  in it. *Excluded Files* lists them.

The panel menu has **Spelling Dictionaries**: it lists the installed dictionaries, imports a Hunspell `.dic`/`.aff` pair
or a LibreOffice extension (`.oxt`) into your own folder (UTF-8 dictionaries only), and manages your word lists (create,
switch on or off, remove words, delete).

Grammar checking is not planned: it needs large models and works well for English only.

## Reports

**Tools → Reports** (++ctrl+shift+r++) opens a window with one tab each for **Files** (folder, name, size, type,
words), **Words** (word, language, count), **Images** (size, times used, resolution), **Style rules** (matches),
**Style classes** (used, unused, not defined), **Characters** (code point, Unicode name, count) and **Links** (internal,
external, broken). Click a column title to sort, type in the filter field to narrow the rows. Double-click a row (or
press ++enter++) to go to its place in the editor; *Next Place* / *Go to Target* visit the other places. *Delete Selected*
(Files) and *Delete Unused* (Images) change the book, with a checkpoint. The save button exports the rows shown as CSV.
Tabs are built when you first open them; *Refresh* rebuilds them after edits.

## Special characters

<figure markdown="1">
![The Insert Special Character window](assets/editor-special-chars-light.png#only-light)
![The Insert Special Character window](assets/editor-special-chars-dark.png#only-dark)
<figcaption markdown="1">Insert Special Character: browse by block or search by name or code; Ctrl+click keeps the window open.</figcaption>
</figure>


In the header bar, open the Markup menu and choose **Insert → Insert Special Character** (++ctrl+alt+i++); it shows every Unicode character by block, with a search by name
or code (`acute`, `U+00E9`), your **favourites** (right-click a character to add, remove or move it) and the characters
you used last. Click inserts the character at the caret, or into the find and replace field when that has the focus, and
closes the window; ++ctrl+click++ inserts and keeps it open. `&`, `<` and `>` are inserted as entities. In a narrow window
(below about 720 px) the list of blocks becomes a drop-down above the characters.

Type a hex code and press ++alt+x++ to turn it into the character (`00e9` or `U+2014`). With **Convert Typed
Entities** switched on (off by default), typing `;` after `&eacute` or `&#233` replaces it by the character; `&amp;`,
`&lt;`, `&gt;`, `&quot;` and `&apos;` are never converted, and only about thirty common names are known.

## Preview

The pane on the right shows the open chapter and follows what you do in the code. The **Preview** button in the
header bar (the second of the group at its end) hides and shows it. The pane has its own small header with a `⋯`
options menu (follow the cursor, reload while typing, scrolling the preview scrolls the code, copy), *Refresh* and
*Find*; the same choices are in Preferences → Editor → Preview. In a window narrower than about 700 px the preview
sits under the code instead of beside it.

![The editor with the preview next to the code](assets/editor-preview-light.png#only-light)
![The editor with the preview next to the code](assets/editor-preview-dark.png#only-dark)

| Control | What it does |
|---|---|
| Follow the cursor | the preview scrolls to the element at the cursor (on by default) |
| Scrolling the code | the preview scrolls to show the same part of the page, also with the mouse wheel, the scrollbar and Page Up/Down |
| Sync scrolling both ways | on by default: scrolling the preview also scrolls the code; switch it off in the preview's options menu (the ⋯ button of the preview header) or in the editor settings; the setting is remembered |
| Click in the preview | puts the cursor at that element in the code |
| Click a link of the book | opens the linked file in the editor and the preview |
| Reload automatically | redraws the preview while you type; switch it off for big files and refresh with ++f5++ |
| Refresh (++f5++) | redraws the preview from the text now |
| Find in the preview | searches the shown text |
| Copy (++ctrl+alt+c++) | copies the text you selected in the preview |

The link between code and preview is made by comparing the text of both, so a block that the preview generates itself
(a list marker, say) has no place in the code, and clicking it does nothing. Between two places that are known, the
scroll position is interpolated, so the panes also agree inside a long paragraph or next to such blocks. A cursor
move (typing, arrow keys, a click) wins for a moment over scrolling, so the preview does not jump away from the
element you are working on. Nothing is synced while the preview is hidden. On vertical pages the preview follows the
cursor but shows no green line.

## Splitting a file

Three ways, all in the Markup menu. Every split takes a checkpoint first; links, anchors, the spine and the contents
are repaired.

![The Markup menu with the split entries](assets/editor-split-menu-light.png#only-light)
![The Markup menu with the split entries](assets/editor-split-menu-dark.png#only-dark)

- **Split at Cursor** cuts the file where the cursor is. A cursor right after a start tag cuts in front of the
  element, so no empty paragraph is left behind. A cursor in the middle of a paragraph splits the paragraph, closing
  and reopening the tags around it.
- **Split by Picking in the Preview**: click the place in the preview where the new file should begin.
- **Split at Several Places** opens a dialog: at headings down to a level, at every element with a class, at
  page breaks (CSS `page-break-before`/`after`), or at the elements a small XPath-like selector picks
  (`//div[@class='part']`, `//section/h2`, `h2.title`). The dialog lists the files that would result, with their first
  heading, before you split.

Only well-formed XHTML can be split.

Two more entries in the Tools menu → Structure: **Split File at Chapter Headings** splits the open chapter at its
headings in one step, and **Merge with Next File** joins the open chapter with the next one in the reading order. In the
Files list you can also select pages and choose *Merge Selected* in the Files menu, or drag a page onto another (see
[File list extras](#file-list-extras)).

## Compare and checkpoints

<figure markdown="1">
![The Checkpoints dialog](assets/editor-checkpoints-light.png#only-light)
![The Checkpoints dialog](assets/editor-checkpoints-dark.png#only-dark)
<figcaption markdown="1">Checkpoints: save a named copy of the whole book, compare it later or go back to it.</figcaption>
</figure>


Every tool takes a snapshot of the book first. Open the Tools menu in the header bar and choose **Checkpoints**:
*Create Checkpoint* makes a named one, *Restore Checkpoint* and *Compare with Checkpoint* list them, and *Manage
Checkpoints* opens the dialog above with time, size and file count of each. Each row has **Compare** (with the book) and a `⋯` menu with *Revert the Book to
This State* (the state before the revert is kept too), *Rename*, *Keep* (the row then says "Kept"; it survives
pruning of old checkpoints) and *Delete*. Delete takes the row away at once and a toast offers **Undo** for ten
seconds; after that it is gone.

The comparison opens in the middle of the editor: two columns with line numbers, only the differences and a few
lines around them (the others are folded), changed lines tinted with the changed words marked. The colours work in
dark mode; with the e-ink theme changes are shown by weight and strike-through instead. Both columns scroll together.

- the file list at the top chooses which changed text file to show; the arrows step through the changes
- *Beautify* pretty-prints the markup first, so formatting changes are not shown (line numbers then refer to the
  pretty-printed text)
- the spin button sets the lines of context; the search box finds text in the differences
- double-click a line to go to it in the code
- Tools → Checkpoints → *Compare with Another Book* compares the open book with another EPUB file; use it to compare with the
  original of a book you converted

Bookx does not keep the original file of an imported book, so the library has no *compare with the original* entry;
keep the original and compare with it as above.

## Live CSS

The *Live CSS* tab (the last icon of the sidebar) shows the styles for the element at the cursor: the rules that match, strongest
first, with the file and line each is written at (click it to jump there), the rules of its parents that it inherits,
and the resulting value of each property. A declaration that another rule overrides is struck through. It reads the
book's stylesheets and `<style>` blocks; the reader's own defaults are not included, sizes in `em` are not converted,
and it is read only: edit the stylesheet itself.

## Tools menu

<figure markdown="1">
![The Tools menu](assets/editor-tools-menu-light.png#only-light)
![The Tools menu](assets/editor-tools-menu-dark.png#only-dark)
<figcaption markdown="1">The Tools menu: Check Book and Fix Problems, the submenus, Spelling, Reports and Checkpoints.</figcaption>
</figure>


The Tools button is in the header bar, between **Preview** and **Markup**; the main menu (☰) is the last button. At
the top of Tools are *Check Book* (++f7++) and *Fix Problems Automatically*; then the submenus *Fix & Clean*, *Styles*,
*Media and Fonts*, *Structure*, *Accessibility* and *Links*; then *Check Spelling*, *Reports* and *Checkpoints*, and
*Plugins* when a plugin adds something. Each tool is listed once.

The entries of the submenus open a dialog that shows **what will change**: press *Preview* and the tool runs on a copy
and lists the files with the lines before and after (the list stays folded until you press it, so the options are
visible first). The tool runs on a checkpoint, so one undo brings the old state back.

### Fix & Clean

| Entry | What it does |
|---|---|
| Reformat Markup | pretty-prints the open file (XHTML, OPF, NCX or other XML), see [Reformat](#reformat-markup) |
| Smarten Punctuation | curly quotes, dashes and ellipses in the text of the pages; the double quotes follow the language rules you choose (English “ ”, German „ “, French « », Swedish ” ”); *Straighten* undoes it; for the open page or all pages. Code, `pre`, scripts, styles and attributes are never touched |
| Fix HTML | repairs pages that are not well-formed XHTML (open page or all pages). If the quick repair cannot fix a page, it is rebuilt with an HTML5 parser, the way a browser reads it; the switch uses that parser always |
| Reformat Markup in Files | tidies the layout of pages, stylesheets and package files (all of them or the open file). Only the white space between block tags changes, so pages look the same |
| Upgrade to EPUB 3 | converts an EPUB 2 book |

### Styles

<figure markdown="1">
![The Transform HTML dialog](assets/editor-transform-html-light.png#only-light)
![The Transform HTML dialog](assets/editor-transform-html-dark.png#only-dark)
<figcaption markdown="1">Transform HTML: build rules such as *Rename the tag*, preview which pages change, then apply.</figcaption>
</figure>


| Entry | What it does |
|---|---|
| Transform Styles | a rule builder: *If a property has a value, change it* (`color` is `red` → `green`), *Set a property everywhere*, *Remove a property* (`margin*` works), *Remove a property when it has a value*, *Multiply all font sizes*, *Make a font family italic*, *Rename a font family*, *Change one colour into another*. Rules apply in order to stylesheets, `<style>` blocks and style attributes. Save a list of rules under a name and load it later (also in other books) |
| Transform HTML | a rule builder for the pages, like *Transform Styles*: *rename a tag* (`b` → `strong`), *remove a tag but keep its content*, *remove an element and its content*, *set*, *remove* or *replace text in an attribute*, *wrap in a new element*, *add*, *remove* or *change a class*. Name the elements with a selector (`p`, `div.note`, `#id`, `//section/p`). Choose all pages or only the open one, press *Preview* to see which pages change, save rule sets under a name. Only the tags you name change: entities, comments, quotes and prefixes such as `epub:type` stay as written. Pages that are not well-formed are skipped and listed |
| Remove Unused CSS Rules | deletes the rules that match nothing in the book (`@font-face` and other @-rules stay) |
| Filter Style Information | removes the ticked properties (with how often they are used) everywhere |

### Media and Fonts

<figure markdown="1">
![The Set Cover dialog](assets/editor-set-cover-light.png#only-light)
![The Set Cover dialog](assets/editor-set-cover-dark.png#only-dark)
<figcaption markdown="1">Set Cover: pick an image of the book, a file, or a cover drawn from the title; optionally add a cover page.</figcaption>
</figure>

<figure markdown="1">
![The Manage Fonts dialog](assets/editor-manage-fonts-light.png#only-light)
![The Manage Fonts dialog](assets/editor-manage-fonts-dark.png#only-dark)
<figcaption markdown="1">Manage Fonts: the fonts embedded in the book, a sample, and *Embed Fonts from this Computer*. A book without embedded fonts shows only that button, on a page of its own.</figcaption>
</figure>


| Entry | What it does |
|---|---|
| Delete Unused Media | lists images, fonts, audio and video that nothing uses, with sizes; the cover is never listed. Unused images are listed and deleted here too |
| Manage Fonts | the fonts embedded in the book with family, style, size, how many characters use them and the licence flag. Open a font to *rename its family* (every `@font-face`, `font-family` and `font` rule and style attribute follows), *remove* it with its `@font-face` rules, or see a **sample** of its real glyphs (edit the sample text). *Embed Fonts from this Computer* opens Embed Referenced Fonts |
| Set Cover | choose an image of the book, a file from the computer or a cover drawn from the title and author; *crop* it to 2:3 or 3:4 and *limit the size* with a live preview. Bookx writes the cover page (or updates the one that exists), marks the image as the cover in the package, the guide and the landmarks, and deletes the old cover image when nothing uses it. *Remove Cover* undoes it. Only PNG, JPEG, GIF and WebP up to 40 MB and 64 megapixels are accepted |
| Embed Referenced Fonts | finds the fonts the pages ask for but the book does not carry among the fonts installed on this computer and embeds them. A font whose licence forbids embedding cannot be ticked; fonts that do not say are marked with a warning. Optionally keeps only the characters the book uses |
| Download External Resources | copies images, stylesheets and media that pages load from the web into the book. Needs a network connection; check the licences |

### Structure and Accessibility

<figure markdown="1">
![The Book Jacket dialog](assets/editor-jacket-light.png#only-light)
![The Book Jacket dialog](assets/editor-jacket-dark.png#only-dark)
<figcaption markdown="1">Book Jacket: a page after the cover that lists the metadata, with the fields you want.</figcaption>
</figure>

The button of the dialog names the action: *Add Jacket*, *Update Jacket* or *Remove Jacket*.

<figure markdown="1">
![The Accessibility Metadata form](assets/editor-a11y-form-light.png#only-light)
![The Accessibility Metadata form](assets/editor-a11y-form-dark.png#only-dark)
<figcaption markdown="1">Accessibility Metadata: a preset or *Suggest from the Book*, then the summary, access modes, features and hazards.</figcaption>
</figure>


| Entry | What it does |
|---|---|
| Set Semantics | says which page is the cover, title page, table of contents, foreword, dedication, index and so on (guide and, in EPUB 3, landmarks) |
| Accessibility Metadata | a form for the EPUB 3 accessibility metadata: summary, access modes, sufficient modes (one block of Textual, Visual, Auditory and Tactile boxes per way of reading the book, and *Add Alternative Set*; the text form is under *Advanced*), features, hazards, conformance statement and the certifier (name, credential, report address). Start from a preset or press *Suggest from the Book*, which looks at the images and their descriptions, audio, MathML, headings and contents. The values are checked against the vocabulary as you type (errors stop *Apply*, warnings do not) and against the book: claiming *alternativeText* while images have no description, or *MathML* without any, is reported. Hazards such as flashing cannot be detected: confirm them yourself |
| Book Jacket | adds, updates or removes a page after the cover that lists the title, authors, series, rating, tags and comments (and optionally the cover and publisher) from the metadata. The page is marked, so Bookx updates it when you change the metadata in the editor and *Remove the jacket* takes it away without a trace |
| Check Accessibility | three tabs: the book’s *Language*; *Image descriptions* (a text per image, *Decorative* for images a screen reader should skip, one text for every use of an image); *Accessibility metadata* (summary, access modes, features, hazards, conformance; EPUB 3) |

### Links

<figure markdown="1">
![The Insert Hyperlink dialog](assets/editor-insert-link-light.png#only-light)
![The Insert Hyperlink dialog](assets/editor-insert-link-dark.png#only-dark)
<figcaption markdown="1">Insert Hyperlink: filter the files of the book, pick an anchor, or type an address.</figcaption>
</figure>


| Entry | What it does |
|---|---|
| Check External Links | visits every web address the book links to, one request every moment, in the background (a progress bar and **Cancel** are in the dialog; Cancel lists what was checked so far); broken ones are listed in the results panel with their page. Nothing is changed. Does not run when the network is switched off |
| Repair Broken Links | fixes links to files or anchors that moved or differ in capital letters; the rest is listed |
| Move Files to a Folder | moves the selected files into a folder of the book and updates every link |

### Other entries

| Entry | What it does |
|---|---|
| Check Book | the full validation, shown in the Problems panel (++f7++), see [Checking the book](#checking-the-book) |
| Fix Problems Automatically | repairs what the checker can fix automatically |
| Insert / Remove Contents Page | in *Structure*: adds or removes a contents page in the book |
| Improve Accessibility | in *Accessibility*: improves accessibility metadata and markup |
| Compress Images | in *Media and Fonts*: shrinks images |
| Shrink Embedded Fonts | in *Media and Fonts*: subsets embedded fonts |
| Arrange Files in Folders | in *Structure*: sorts files into folders by type |
| Split File at Chapter Headings | in *Structure*: splits the open chapter; links, contents and manifest are repaired |
| Merge with Next File | in *Structure*: merges the open chapter with the next one |
| Add Narration / Remove Narration | in *Media and Fonts*: reads the book aloud into the EPUB as a media overlay, see [Read aloud](read-aloud.md#narrating-a-book-media-overlays). The progress dialog has **Run in Background**; if you edit the book while it runs, the narration is not added (a message says so) |
| Check Spelling | the spelling panel, see [Spelling](#spelling) |
| Reports | the reports window, see [Reports](#reports) |
| Checkpoints | Create, Restore and Compare with a checkpoint, *Manage Checkpoints*, *Compare with Another Book*, see [Compare and checkpoints](#compare-and-checkpoints) |
| Plugins | transforms and actions of [plugins](plugins-and-patches.md); one checkpoint each |

++f8++ and ++shift+f8++ still go to the next and the previous problem.

### Reformat Markup

*Reformat Markup* in *Fix & Clean* pretty-prints the open XHTML, OPF, NCX or XML file. *Reformat Markup in Files*
does the same for all pages, stylesheets and package files, or for the open file only. (Both were called *Beautify*.)

## File list extras

<figure markdown="1">
![The Rename Tag dialog](assets/editor-rename-tag-light.png#only-light)
![The Rename Tag dialog](assets/editor-rename-tag-dark.png#only-dark)
<figcaption markdown="1">Rename Tag changes an element and its closing tag together.</figcaption>
</figure>

<figure markdown="1">
![The Bulk Rename dialog](assets/editor-bulk-rename-light.png#only-light)
![The Bulk Rename dialog](assets/editor-bulk-rename-dark.png#only-dark)
<figcaption markdown="1">Bulk Rename numbers the selected files in reading order and fixes the references.</figcaption>
</figure>


- **Order and groups.** Pages are listed in reading order, then stylesheets, images, fonts and package files, each
  alphabetically. A number shows the place in the reading order.
- **Hover** a file for its path, size, type and what is special about it. Icons mark the package document, the
  contents (NCX or navigation document), the cover, the title page, files not listed in the package, pages outside the
  reading order, pages listed twice and pages that are not part of the main reading order.
- **Reorder by dragging.** Drag a page row onto another: a line shows whether it lands above or below. The buttons
  below the list still move a page one place.
- **Merge by dragging.** Drop a page onto the *middle* of another page (the row is highlighted instead of a line): Bookx
  asks, then adds the dragged page to the end of the other one, repairs the links to it and removes it. It runs on a
  checkpoint, so it can be undone. Only pages can be merged this way.
- **The rest of the Files menu** (the `⋯` button under the file list, or a right-click on a file). *Select* all text files, stylesheets, images or fonts, or none. *Use as Cover
  Image*, *Cover Page*, *Title Page* or *Table of Contents*. *Toggle Not in Main Reading Order*. *Change Extension*.
  *Move to Folder*. *Replace with File* (links follow when you change the name). *Open With* opens the file in
  another program and offers to update the book when you save it there. *Browse Images* shows thumbnails with their
  sizes and which are unused. *Show Full Paths*.
- **Your own templates.** Put `.xhtml`, `.html` or `.css` files in the `editor-templates` folder of Bookx’s
  configuration folder; they appear in *New File* (`{title}` in the text is replaced by the name you type).

## Book menu

<figure markdown="1">
![The Book menu of the editor](assets/editor-book-menu-light.png#only-light)
![The Book menu of the editor](assets/editor-book-menu-dark.png#only-dark)
<figcaption markdown="1">The main menu: save a copy, open another book, revert, wrapping, preferences and help.</figcaption>
</figure>


The ☰ button at the right end of the header bar; ++f10++ opens it. At most 12 entries.

| Entry | Shortcut | What it does |
|---|---|---|
| Save a Copy | ++ctrl+shift+s++ | asks for a file name and writes the book as it is now (unsaved edits included) as an EPUB file; the library is not changed |
| Save a Copy and Edit It | ++ctrl+alt+shift+s++ | the same, adds the copy to the library and continues editing the copy |
| Open Another Book | ++ctrl+o++ | shows another book of the library in this window |
| New Book | | a book with one empty chapter |
| More Ways to Open | | *Open an Unzipped Folder* (a folder with an unzipped EPUB, added to the library); *Open a Book of Another Format to Edit* (an AZW3, MOBI, KEPUB, FB2, FBZ, DOCX, ODT, RTF, HTMLZ or TXTZ file is converted to a temporary EPUB in memory, a KEPUB is edited as it is with its Kobo markup, and nothing is added to the library; a banner says you are editing a copy; *Save* asks whether to **save as an EPUB** (the safe choice) or to **convert back** into the original format, the old file is kept next to it as a backup (`book.azw3.bak`); a warning names what may be lost; formats Bookx cannot write, ODT and FBZ, can only be saved as EPUB); *Import as New Book* (an HTML, DOCX or text file) |
| Revert to Before the Last Change / Revert to After (Redo) | ++ctrl+alt+z++ / ++ctrl+alt+shift+z++ | walks back and forth over the checkpoints (a new change ends the redo list); greyed out while there is nothing to revert or redo |
| Wrap Long Lines | | wraps long lines in the code view |
| Preferences | ++ctrl+comma++ | the Bookx Preferences window; its **Editor** page holds the editor settings |
| Keyboard Shortcuts | ++ctrl+question++ | every shortcut, editable (the Keyboard Shortcuts page of Preferences) |
| Help | ++f1++ | opens this page in your browser |
| About Bookx | | version and credits |

*Close Other Tabs* and *Close Tabs to the Right* (++ctrl+shift+w++ / ++alt+shift+w++) are in the context menu of a tab
(right-click it). *Reopen tabs and cursors* (open tabs, the selected tab and the caret of each file, remembered per book)
is a switch in Preferences → Editor.

A save that fails shows a banner at the top of the window ("Could not save the book") with **Try Again**; it stays
until the book is saved.

### Small windows

Below about 1000 px the file sidebar slides over the code (a button at the start of the header shows it) and the minimap
goes; below about 700 px the preview sits under the code instead of beside it. In a narrow window the spelling panel
replaces the preview while it is open. The find bar is one row: the **Search Options** button holds the mode, match
case, whole words and scope; ++ctrl+h++ adds the replace row. The sidebar tabs show icons only (their names are the
tooltips).

Opening a book in the editor from a terminal: `bookx --edit book.epub OEBPS/ch1.xhtml --select-text "some words"` or
`bookx-cli edit book.epub OEBPS/ch1.xhtml --select-text "some words"`. The file is added to the library if it is not
there; the named files open in tabs and the text is selected in the last one.

## Questions and answers

Short how-tos for the jobs people ask about most. Menu names are the ones in the editor's header bar: **Tools**,
**Markup** and the main menu (☰, called the Book menu above); the **Files** tab is the first icon of the sidebar.

### Fonts and styles

??? question "How do I change an embedded font?"
    1. Open the Tools menu in the header bar → **Media and Fonts → Manage Fonts**. It lists the fonts the book carries.
    2. To keep the font but call it something else, open it and use *rename its family*: every `@font-face`,
       `font-family` and `font` rule and style attribute follows.
    3. To swap in a different font file, go to the **Files** tab, right-click the font file (listed under the fonts),
       choose **Replace with File**, pick the new font and confirm the name it keeps in the book (links follow if you
       change it). If the new file has another format (`.otf` instead of `.ttf`), check the `src` and `format()` of
       its `@font-face` rule in the stylesheet.
    4. To embed a font that is installed on this computer, use Tools → Media and Fonts → **Embed Referenced Fonts**.
       A font whose licence forbids embedding cannot be ticked.
    5. To remove a font, open it in Manage Fonts and choose *remove*; its `@font-face` rules go with it.

??? question "How do I change the font across the whole book?"
    1. Tools → **Styles → Transform Styles**, then add a rule.
    2. *Rename family* turns every use of one font name into another (`Georgia` → `Merriweather`).
       *Set property* with the property `font-family` gives every `font-family` declaration the value you type.
    3. Press **Preview** to see which files change, then **Apply**. One checkpoint is taken first.
    4. *Set property* only changes declarations that already exist. If the book never sets a font, add a rule such as
       `body { font-family: serif; }` to its stylesheet (open the `.css` file in the Files tab and type it).
    5. To change only how *you* read the book, use the reader's text appearance settings instead; they do not touch the
       file.

??? question "How do I make all text bigger or smaller?"
    Tools → Styles → **Transform Styles** → add the rule *Scale font sizes* and give a factor such as `1.2`.
    Preview, then Apply.

??? question "How do I change one CSS rule?"
    1. In the **Files** tab click the `.css` file; it opens in the code view. Edit, then ++ctrl+s++. The preview follows as
       you type.
    2. If you do not know which rule styles an element, open the chapter, put the caret in the element and look at the
       **Live CSS** tab (last icon of the sidebar): it lists the matching rules with the file and line of each. Click a
       line to jump to it.
    3. To rename a class everywhere, right-click it in the code and choose **Rename Class**.

??? question "How do I change the whole stylesheet?"
    - Edit it by hand: open the `.css` file and select all (++ctrl+a++), paste your version, save.
    - Or replace the file: right-click it in the Files tab → **Replace with File** and choose the new `.css`.
    - For rule-based changes (colours, margins, removing a property everywhere) use Tools → Styles → **Transform
      Styles**; *Remove Unused CSS Rules* cleans up what nothing uses.

??? question "How do I link a stylesheet to my pages?"
    1. If the stylesheet is not in the book yet, add it (see "How do I add a new asset" below) or create it with **New
       File** and the stylesheet template.
    2. In the Files tab select the pages and the stylesheet, open the `⋯` menu (or right-click) and choose **Link
       Stylesheets**.
    3. Tick *Remove existing stylesheet links first* if the pages should use only the new sheet.
    A new page made with **New File** can be linked at once with its *Link the book's stylesheets* box.

### Files and assets

??? question "How do I add a new file (chapter or page)?"
    1. Open the Files tab, then the `⋯` menu → **New File** (++ctrl+n++).
    2. Type a file name, pick a template (XHTML page or stylesheet, or one of your own) and press *Create*.
    3. The page is added to the reading order after the file you have open. Move it with the up and down buttons under
       the file list.
    4. Add it to the table of contents in the **Contents** tab if readers should see it there.

??? question "How do I add a new asset (image, font, stylesheet, audio)?"
    1. In the Files tab open the `⋯` menu → **Add Files** and choose one or more files from your computer.
    2. Bookx puts each file in the folder that suits its type and lists it in the book's package, so no manual
       bookkeeping is needed.
    3. Use it: for an image, put the caret where it belongs and choose Markup → **Insert → Insert Image**
       (++ctrl+alt+m++). A stylesheet is attached with *Link Stylesheets*. A font needs an `@font-face` rule that points at
       it. Audio and video have no dialog; type the `<audio>` or `<video>` tag yourself, and completion offers the file
       names inside `src`.

??? question "How do I rename a file and keep the links working?"
    Select the file in the Files tab and press ++f2++ (or **Rename** in the `⋯` menu). Every `href`, `src`, CSS `url()` and
    `#fragment` that pointed at it is rewritten. For many files at once use **Bulk Rename**, which numbers the selected
    files in reading order and repairs the references too.

??? question "How do I delete a file?"
    Select it and press ++delete++ (or **Delete Selected** in the `⋯` menu). A file nothing refers to goes at once and a
    toast offers **Undo** for about ten seconds. Several files, or a file something still points to, ask first. If links
    broke, run Tools → **Links → Repair Broken Links** or check the book (++f7++).

??? question "How do I remove files nothing uses?"
    Tools → **Media and Fonts → Delete Unused Media** lists unused images, fonts, audio and video with their sizes (the
    cover is never listed). Tools → **Styles → Remove Unused CSS Rules** removes style rules that match nothing. The
    Check Book findings also offer *Fix* for unused files.

??? question "How do I move files into folders?"
    Select the files in the Files tab and choose **Move to Folder** in the `⋯` menu, or use Tools → **Links → Move Files
    to a Folder**. Links are updated. Tools → **Structure → Arrange Files in Folders** sorts the whole book by type.

### Cover, contents and metadata

??? question "How do I change the cover?"
    - Tools → **Media and Fonts → Set Cover**: pick an image of the book, a file from the computer, or a cover drawn from
      the title and author; crop it and limit its size, then press *Set Cover*. *Remove Cover* is in the same dialog.
    - Or right-click an image in the Files tab → **Use as Cover Image**, or open the image and press **Use as Cover**
      under the picture.

??? question "How do I edit the table of contents?"
    1. Open the **Contents** tab (second icon of the sidebar).
    2. Type in an entry to rename it, drag the grip at its left to move it (the middle of another row makes it a
       sub-entry), or use the *Move entry* menu or ++alt+up++ / ++alt+down++ / ++alt+left++ / ++alt+right++.
    3. *Add* puts a new entry after the selected one; *Delete* removes one (an **Undo** toast appears).
    4. To rebuild everything, press *Generate*: from the headings, from the links of a contents page, or one entry per
       file.
    5. Save with ++ctrl+s++; `nav.xhtml` and `toc.ncx` are both written.

??? question "How do I add a contents page that readers see inside the book?"
    Tools → **Structure → Insert Contents Page** adds one; *Remove Contents Page* takes it away.

??? question "How do I change the title, author or other metadata?"
    Open the **Metadata** tab (third icon of the sidebar) and press **Edit Metadata**. The change is written to the file
    when you save.

### Checking and fixing

??? question "How do I fix the Problems the checker reports?"
    1. Press ++f7++ or choose Tools → **Check Book**. The **Problems** panel opens at the bottom.
    2. Click a problem to read what it means and how to fix it; double-click it (or ++enter++) to jump to the place.
    3. Press **Fix** for that problem or **Fix all of this kind** for the whole book. Only problems with a Fix button can
       be repaired automatically; a checkpoint is taken first.
    4. Tools → **Fix Problems Automatically** repairs everything the checker can fix in one go.
    5. For the rest, edit by hand; ++f8++ and ++shift+f8++ step through the problems. Use *Ignore this kind* for
       findings you do not care about (not for broken markup).

??? question "How do I tidy messy markup?"
    Tools → **Fix & Clean → Reformat Markup** pretty-prints the open file. *Reformat Markup in Files* does it for all
    pages, stylesheets and package files, and *Fix HTML* repairs pages that are not well-formed.

??? question "How do I find and repair broken links?"
    Tools → **Links → Repair Broken Links** fixes links to files or anchors that moved or differ in capitals and lists
    the rest. **Check External Links** visits every web address in the background and lists the broken ones. The
    **Reports → Links** tab lists internal, external and broken links.

??? question "How do I add alternative text to images?"
    Tools → **Accessibility → Check Accessibility**, tab *Image descriptions*: type a text per image, or tick
    *Decorative* for images a screen reader should skip.

### Structure and history

??? question "How do I split a chapter?"
    - Put the caret where the new file should start, open the Markup menu and choose **Split at Cursor**.
    - Or choose **Split by Picking in the Preview** and click the place in the preview.
    - Or **Split at Several Places** to split at every heading down to a level, at page breaks or at elements you name.
    - Tools → **Structure → Split File at Chapter Headings** does the common case in one step.
    Links, the reading order and the contents are repaired, and a checkpoint is taken first.

??? question "How do I merge chapters?"
    Open the first one and choose Tools → **Structure → Merge with Next File**. To merge several, select the pages in the
    Files tab and choose **Merge Selected** in the `⋯` menu, or drop one page onto the middle of another.

??? question "How do I undo a tool run or restore a checkpoint?"
    - Right after a tool, a toast says what changed; press **Undo** in it (about ten seconds).
    - Later, open the Book menu (☰) and choose **Revert to Before the Last Change** (++ctrl+alt+z++); **Revert to After
      (Redo)** goes forward again.
    - For a named state, Tools → **Checkpoints → Restore Checkpoint**, or **Manage Checkpoints** for the full list with
      *Compare*. *Create Checkpoint* makes one yourself before something risky.
    - The header's Undo and Redo buttons only undo typing.

??? question "How do I see what I changed?"
    Tools → **Checkpoints → Compare with Checkpoint** shows the differences between the book and a checkpoint. The narrow
    column left of the line numbers also marks new and changed lines (green and blue) since the last checkpoint.

### Finding, saving and exporting

??? question "How do I find and replace across the whole book?"
    1. Press ++ctrl+h++ (find with the replace row), type the text and the replacement.
    2. Open **Search Options** at the end of the find bar and set the *scope* to *All files* (or *Text files* for pages
       only).
    3. *Find All* lists every match, *Count* counts them, **Replace All** replaces them after taking a checkpoint; the
       toast has **Undo**.
    More in [Search and replace](search-and-replace.md).

??? question "How do I save a copy or export to another format?"
    - Book menu (☰) → **Save a Copy** (++ctrl+shift+s++) writes the book as it is now to an EPUB file you choose; the
      library keeps its own version. **Save a Copy and Edit It** also adds the copy to the library and continues with it.
    - The editor writes EPUB only. For KEPUB, AZW3, MOBI and the other formats, press **Save**, then in the library
      right-click the book → **Convert To**. See [Converting and exporting](converting.md).

??? question "How do I open a MOBI, AZW3, FB2 or Word file in the editor?"
    Book menu (☰) → **More Ways to Open → Open a Book of Another Format to Edit**. It is converted to a temporary EPUB;
    a banner says you are editing a copy, and **Save** asks whether to save as EPUB or convert back.

??? question "What if saving fails?"
    A banner "Could not save the book" stays at the top with **Try Again** until the book is saved. Your edits are not lost.

### Working in the window

??? question "How do I see the preview next to the code?"
    Press the **Preview** button in the header bar (it shows or hides the pane). The preview scrolls with the code, and a click
    in it puts the cursor at that element. Below about 700 px of window width it sits under the code instead. Its `⋯`
    menu holds *follow the cursor*, *reload while typing* and *scroll the code with the preview*.

??? question "How do I turn on spell check and add a word?"
    - Misspelled words are underlined in red while you type. If they are not, switch on *Underline misspelled words* in
      Preferences → Editor, and install a Hunspell dictionary for the book's language on your system (none is bundled).
    - Right-click a word for **Replace with**, **Ignore** or **Add to Dictionary**.
    - Tools → **Check Spelling** opens a panel with every misspelled word of the book and *Change All*.

??? question "How do I open the command palette?"
    Press ++ctrl+shift+p++ and type a few letters; it finds editor commands, every item of the Tools, Markup and Files menus
    and the files of the book. (++ctrl+k++ is *Insert Hyperlink* in the editor; it opens the palette in the library and
    reader.)

??? question "How do I change editor settings (font, colours, tabs)?"
    Book menu (☰) → **Preferences** (++ctrl+comma++) → the **Editor** page: code font and size, tab width, line numbers,
    colour scheme, closing tags, completion, spelling underlines and the preview. The command palette's *Editor
    Preferences* opens the same settings in a small window.

??? question "How do I change a keyboard shortcut?"
    Book menu (☰) → **Keyboard Shortcuts** (++ctrl+question++), group *Editor*. Menu items show the new key at once.

??? question "How do I insert a link, an image or a special character?"
    Markup menu → **Insert**: *Insert Hyperlink* (++ctrl+k++), *Insert Image* (++ctrl+alt+m++), *Insert Special
    Character* (++ctrl+alt+i++), or type a hex code and press ++alt+x++. *Manage Snippets* is in the same place.

??? question "How do I work in a small window?"
    Below about 1000 px the sidebar hides; the button at the start of the header bar slides it over the code. Below
    about 700 px the preview moves under the code.

!!! note "Limitations"
    - The editor works on EPUB. Other formats are converted to EPUB first; see *Open a Book of Another Format to Edit* for the way back.
    - Converting back is a conversion: styling, layout and format-specific features can be lost. Keep the backup until you have checked the result.
    - Fonts are searched in the background in the system font folders and `~/.local/share/fonts`; font files over 12 MB are skipped.
    - Link checking and downloading use plain web requests, one at a time; some sites refuse them.
    - Check Spelling and the underlines in the code view need system Hunspell dictionaries; none are bundled.
    - The EPUB validator is Bookx's own rule set, in pure Rust. It is compared with the `epubveri` validator in
      tests, but it is not the reference EPUBCheck and can be stricter.
