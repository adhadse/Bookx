# Search and replace

The book editor's find bar searches one file or the whole book. To open it, press ++ctrl+f++ in the editor, or press
the **Find** button at the end of the header bar; ++ctrl+h++ opens it with the replace row. The bar has several modes,
scopes of files, saved searches, and a **function mode** in which a small script decides every replacement.

Every *Replace All* first takes a checkpoint named "Before: Replace All" (see [Checkpoints](editing.md#compare-and-checkpoints) to go back).

## The find bar

The bar sits above the code and is one row high: the *Find* box, previous and next, *Find All*, *Count*, a count of
matches and the **Search Options** button (a cog) at its end. ++ctrl+h++ adds a second row with the *Replace with* box, *Replace* and
*Replace All*. The close button at its right end hides it again.

Everything that is a setting rather than an action lives in the **Search Options** popover: the mode, *Aa*, *W*,
*.⏎*, *<>*, the scope, the function list (in Function mode), the special-character button and a `⋯` button with *Saved
Searches*, *Save This Search*, *Functions* and *Replace & Find Previous*.

| Control | What it does |
|---|---|
| Find / Replace boxes | The clock button next to each lists the last 20 entries. They are remembered between sessions. |
| ++enter++, ++f3++, ++ctrl+g++ | Next match. ++shift+f3++ or ++ctrl+shift+g++ goes to the previous one. The search wraps around; the bar says "wrapped around". |
| *Replace* (++ctrl+r++, `Ctrl+]`) | Replaces the selected match and selects the next one. `Ctrl+[` (or the menu's *Replace & Find Previous*) replaces and goes backwards. |
| *Replace All* | Replaces in the whole scope. |
| *Find All* | Lists every match of the scope; click one to jump to it. |
| *Count* | Counts the matches over the whole scope and says "12 matches in 3 files" in the bar. Hover the text for the count per file. |
| *Aa* (Search Options) | Match case. |
| *W* (Search Options) | Whole words only. |
| *.⏎* (Search Options) | Dot matches line breaks (Regex and Function modes). |
| *<>* (Search Options) | Ignore HTML markup (see below). |
| Insert character button (Search Options) | Special characters (no-break space, dashes, quotes) and, in regex mode, escapes such as `\n`, `\t`, `\s`. |

All matches in the open file are highlighted while you type, and "n of m in this file" is shown in the bar.
Saved searches (below) keep a search with all its settings for later.

## Modes

Choose the mode in the first dropdown of the Search Options popover.

**Text** searches for exactly what you type.

**Regex** is a regular expression with look-ahead and look-behind, back-references (`\1` in the pattern) and named
groups. In the *Replace with* box use `\1`, `\g<name>` or `$1` for groups, `\n` for a line break, `\t` for a tab and
`\\` for a backslash. If a replacement should contain a `$`, write `\$`.

**Function** is a regular expression whose replacement is computed by a function; see [Function mode](#function-mode).

**Fuzzy** finds text that is *almost* what you typed: a missing, extra or different letter still matches (`recieve` is found by `receive`). Short words (up to 3 letters) must match exactly; the allowed mistakes grow with the length (one up to 6 letters, then one more per 4). Case, whole word, the marked-text scope and Replace All work as in the other text modes; the replacement is inserted as typed.

**Tag** finds start tags by element name and attributes (see below).

### Ignore HTML markup

With *<>* on (Text and Regex modes) Bookx searches the text of the page as a reader sees it. A phrase that is split by
inline tags is found:

```text
so on <i>a</i> word here        ← find "on a word"
```

The match is `on <i>a</i> word`. The rules:

- Tags do not take part in the match. A block tag (`<p>`, `<div>`, `<br/>`, headings, list items …) counts as one line
  break, so a match cannot run from one paragraph into the next by accident (use `\n` in Regex mode if you want that).
- Character references are decoded: `&` finds `&amp;`.
- Text in `<head>`, `<script>` and `<style>` is not searched.
- **Replacing** writes the new text where the match starts, deletes the matched text in the later pieces, and **keeps
  every tag**. Replacing `on a word` with `X` gives `X<i></i> here`. The empty `<i></i>` stays. The replacement text is escaped (`<` becomes `&lt;`).
- A line break contributed by a block tag is never deleted.
- It cannot be combined with *Marked text*.

### Tag search

Choose *Tag* in the mode dropdown of the Search Options popover, then type a query in the Find box. The syntax is a small part of CSS selectors:

| Query | Matches |
|---|---|
| `img` | every `<img>` |
| `img[alt=""]` | images with an empty `alt` |
| `a[href^=http]` | links whose `href` starts with `http` (`$=` ends with, `*=` contains, `~=` contains the word, `!=` differs) |
| `*[class~=note]` | any element with the class `note` |
| `a[href][!title]` | links that have an `href` and no `title` |
| `img[alt="Cover" i]` | the value is compared ignoring case |

Names are not case sensitive; namespace prefixes are ignored. Matching start tags are highlighted and listed by
*Find All*.

The **Replace with** box then changes the attributes of every matching tag:

| Replace with | Effect |
|---|---|
| `alt=Picture` | sets (or adds) the attribute `alt` |
| `-title` | removes the attribute `title` |
| `=https://` | replaces the part of the attribute the query matched: the whole value for `=`, the start for `^=`, the end for `$=`, every occurrence for `*=` and `~=` |

Example: `a[href^=http://]` with `=https://` turns every `http://` link into an `https://` link.

## Scopes

The scope is the *Search in* dropdown of the Search Options popover.

| Scope | Files |
|---|---|
| This file | the open file |
| Text files | the XHTML/HTML files |
| Style files | the stylesheets (`.css`) |
| All files | every text file: XHTML, CSS, XML, OPF, NCX, SVG |
| Selected files | the files selected in the Files tab of the sidebar (hold ++ctrl++ or ++shift++ to select several) |
| Open files | the files that have a tab |
| Marked text | only the regions you marked with ++ctrl+shift+m++ (or *Mark Selection* in the command palette); **Clear Marked Text** (command palette, ++ctrl+shift+p++) removes the marks of all files; ++ctrl+shift+m++ with nothing selected clears the open file. Marks are tinted in the text and in the overview map and stay when you switch tabs |

## Saved searches

Open Search Options and press its `⋯` button → **Save This Search** stores the find and replace texts, the mode, the
options, the scope and, in Function mode, the function. **Saved Searches** opens the list:

- *Add Current* saves the bar's search; *Edit* changes any part; *Delete*, *Up* and *Down* manage the list.
- *Load* (or double-click) puts the search into the find bar.
- Select several and press **Count** or **Replace All** to run them one after the other, in list order. Each search
  sees the result of the one before. One checkpoint is taken first. A search that fails (for example an invalid
  expression) is reported and the others still run. The scope *Marked text* cannot be run from the list.
- The import and export buttons read and write a JSON file, so searches can be shared. Imported searches whose
  name is taken are renamed.

They are stored as `saved-searches.json` next to your settings.

## Function mode

In Function mode the replacement is the result of a function written in [Rhai](https://rhai.rs), a small scripting
language that looks like Rust and JavaScript. (Calibre uses Python for this; Bookx does not, and functions are not
interchangeable.) Choose *Function* in the mode dropdown, choose a function in the list that appears next to it, type the regular expression whose matches it
should handle, and press *Replace All*.

A function looks like this:

```rhai
fn replace(m, number, file, meta, data) {
    m.group(1).to_upper()
}
```

| Argument | Meaning |
|---|---|
| `m` | the match: `m.group(1)`, `m.group("name")` (`()` if the group took no part), `m.text`, `m.start`, `m.end` (byte positions), `m.groups` |
| `number` | 1 for the first match, 2 for the next … counted across the whole Replace All, all files included |
| `file` | the file's path in the book, empty for marked text |
| `meta` | the book's data: `meta.title`, `meta.authors` (a list), `meta.language`, `meta.publisher`, `meta.series`, `meta.series_index`, `meta.tags`, `meta.identifiers.isbn`, `meta.description`, `meta.rights` |
| `data` | a store that lives for one Replace All: `data["count"] = 1;` and `data["count"]` (`()` if not set) |

Return the replacement text. Return `()` to leave the match as it is. A number is turned into text.

**Calling other functions.** Give `replace` a sixth parameter, `functions`, to use your other saved functions:

```rhai
fn replace(m, number, file, meta, data, functions) {
    functions["Title Case"].invoke(m, number, file, meta, data) + "!"
}
```

`invoke` takes the same five values as `replace` and returns what that function returns; both share `data`. A function
that is called this way cannot call further functions. An unknown name stops the run with a message.

Helpers: `print(x)` (see *Debug output*), `dict_recognized(word)` (is the word in the book's spelling dictionary, the one
used by Check Spelling), `text_upper(s)`, `text_lower(s)`, `text_capitalize(s)`, `text_swap_case(s)`,
`text_title(s)` (these leave tags alone), `smarten_hyphens(s)`, `strip_tags(s)`, `escape_html(s)`.

**Options** are comments at the top of the function:

```rhai
// @file_order spine           (or spine-reverse) visit the files in reading order instead of the file list's
// @call_after_last_match      call the function once more after the last match, with m = ()
// @suppress_result_dialog     do not show the results window after the run
// @append_final_output_to_marked   add the final output on a new line after the marked text and mark it
// @persistent_data            keep `data` between runs (kept per library)
```

The extra call returns a text that is shown as *Final output* in the results window, with a button to copy it.

**Debug output**: everything the function `print`s is shown in a *Function results* window after the run (not when
*suppress_result_dialog* is set, unless the function failed). If the function fails, nothing is changed in the book:
the window shows the error and the line of the script it happened on.

### The sandbox

A function cannot read or write files, use the network, load modules or call `eval`. One call may run at most 500,000
operations and a whole run at most 30 seconds; strings, lists and maps have size limits; recursion depth is limited.
An endless loop or a huge allocation stops the run with an error message and changes nothing.

### Writing and testing functions

Search Options → `⋯` → **Functions** lists the built-in functions and yours. *Use* selects one in the find bar (and fills in its
suggested expression if the Find box is empty). *Edit* opens the **function editor** with a source editor,
the suggested regular expression, and a **Test on Sample** button. The sample is the current selection (or the first part
of the open file); the test shows how many replacements were made, the first changes as *before → after*, the debug output
and the resulting text. Nothing in the book changes. Errors are shown with their line number and the editor jumps
to that line. *Save* refuses a function that does not compile.

Your functions are stored as `search-functions.json`; the import and export buttons share them. A function saved
with the name of a built-in one replaces it.

### Five worked examples

All five ship as built-in functions.

**1. Title-case headings, leaving the tags alone.** Expression: `(?s)<h[1-6][^>]*>.*?</h[1-6]>`

```rhai
fn replace(m, number, file, meta, data) {
    text_title(m.text)
}
```

`<h1 class="big">the war of <i>the</i> worlds</h1>` becomes `<h1 class="big">The War of <i>the</i> Worlds</h1>`. Small
words (a, an, the, of, in, on …) stay lower case except at the start and the end, and words with capitals inside
(NASA, iPhone) are kept.

**2. Smarten hyphens.** Expression: `(?s)(?<=>)[^<]+(?=<)`

```rhai
fn replace(m, number, file, meta, data) {
    smarten_hyphens(m.text)
}
```

`wait---what--no` becomes `wait—what–no`; a hyphen with spaces around it becomes a spaced en dash. Hyphens inside
tags and attributes are not touched.

**3. De-hyphenate with the dictionary.** Expression: `(\w+)-\s*\n\s*(\w+)`

```rhai
fn replace(m, number, file, meta, data) {
    let joined = m.group(1) + m.group(2);
    if dict_recognized(joined) {
        joined
    } else {
        print("kept: " + m.text);
        m.text
    }
}
```

`exam-⏎ple` becomes `example` when the dictionary knows it; `foo-⏎bar` stays, and appears in the debug output. Needs a
spelling dictionary for the book's language (see [Editing](editing.md) for spell checking).

**4. Auto-number sections.** Expression: `(<h2[^>]*>)(.*?)(</h2>)`

```rhai
fn replace(m, number, file, meta, data) {
    m.group(1) + number + ". " + m.group(2) + m.group(3)
}
```

Every `<h2>` gets "1. ", "2. " … in front, counting on through all files. Combine it with
`// @file_order spine` so the numbers follow the reading order.

**5. Generate a table of contents from headings.** Expression: `<h2[^>]*\bid="([^"]+)"[^>]*>(.*?)</h2>`

```rhai
// @call_after_last_match
fn replace(m, number, file, meta, data) {
    if m == () {
        return "<ol>\n" + data["toc"] + "</ol>";
    }
    if data["toc"] == () { data["toc"] = ""; }
    data["toc"] += "<li><a href=\"" + file + "#" + m.group(1) + "\">" + strip_tags(m.group(2)) + "</a></li>\n";
    ()
}
```

For every match the function adds a list item to `data["toc"]` and returns `()`, so the text is not changed. The extra
call after the last match returns the finished list, which the results window shows as *Final output*; copy it into a
contents page. (The contents tab's *Generate* does this without a script; the example shows the technique.)

## Shortcuts

| Action | Shortcut |
|---|---|
| Open find / open find with the replace row | ++ctrl+f++ / ++ctrl+h++ |
| Next / previous match | ++f3++ or ++ctrl+g++ / ++shift+f3++ or ++ctrl+shift+g++ |
| Replace and find next / previous | ++ctrl+r++ or `Ctrl+]` / `Ctrl+[` |
| Go to line | ++ctrl+l++ |
| Mark selected text | ++ctrl+shift+m++ |

## Questions and answers

??? question "How do I find and replace across the whole book?"
    1. Press ++ctrl+h++ in the editor, type the text and the replacement.
    2. Press the Search Options button (the cog at the end of the first row) and set *Search in* to *All files*, or *Text
       files* to leave stylesheets and package files alone.
    3. Check with *Find All* (a list of every match; click one to jump) or *Count*, then press **Replace All**.
    4. A checkpoint "Before: Replace All" is taken first and the toast has an **Undo** button.

??? question "How do I search only some files?"
    Select the files in the **Files** tab (Ctrl or Shift-click), then set *Search in* to *Selected files*. *Open files*
    searches the files that have a tab, *Style files* the stylesheets only.

??? question "How do I search only part of a file?"
    Select the text and press ++ctrl+shift+m++ to mark it (repeat for other places, in any file), then set *Search in* to
    *Marked text*. ++ctrl+shift+m++ with nothing selected clears the marks of the open file; *Clear Marked Text* in the
    command palette (++ctrl+shift+p++) clears all of them.

??? question "How do I use a regular expression?"
    Open Search Options and choose *Regex* in the mode dropdown. In the *Replace with* box use `\1` or `$1` for groups
    and `\n` for a line break. The insert button in the same popover lists the escapes such as `\t` and `\s`.

??? question "How do I find text that is split by italic or bold tags?"
    In Search Options (Text or Regex mode) switch on `<>` (ignore HTML markup). The phrase is found even when a `<i>` or
    `<b>` sits in the middle; replacing keeps the tags.

??? question "How do I change an attribute in many places, such as http to https links?"
    Choose *Tag* mode, type `a[href^=http://]` in Find and `=https://` in *Replace with*. `alt=Picture` sets an attribute
    and `-title` removes one.

??? question "How do I keep a search and run it again later?"
    Search Options → `⋯` → **Save This Search**, give it a name. **Saved Searches** in the same menu lists them: load one,
    or select several and press *Count* or *Replace All* to run them in order (one checkpoint is taken first).

??? question "How do I search with a script?"
    Choose *Function* mode, pick a function in the list next to the mode (five come built in), write the regular
    expression in Find and press *Replace All*. Search Options → `⋯` → **Functions** opens the list where you write
    your own and test them on a sample before using them.

??? question "How do I jump to the next or previous match?"
    ++enter++, ++f3++ or ++ctrl+g++ go forward; ++shift+f3++ or ++ctrl+shift+g++ go back. The search wraps around.

??? question "How do I undo a Replace All?"
    Press **Undo** in the toast that appears (about ten seconds). Afterwards, open the main menu (☰ at the end of the
    header bar) and choose *Revert to Before the Last Change* (++ctrl+alt+z++), or Tools → Checkpoints → *Restore
    Checkpoint*.

??? question "How do I go to a line number?"
    Press ++ctrl+l++ and type the number.

??? question "Can I search the text of all my books, not only the open one?"
    Not from the editor's find bar, which works inside the open book. To find which books contain a phrase, use the
    `text:` search word in the library's search box; see [Search](library.md#search).

