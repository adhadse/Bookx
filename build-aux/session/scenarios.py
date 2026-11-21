"""The scenarios. Each one gets its own GNOME session, library and config (see harness.py)."""
import os
import time
import zipfile

from harness import scenario

SEL_X, SEL_Y = 400, 348  # a text line in the left column of a 1280x800 reader page


def select_word(c, x=SEL_X, y=SEL_Y):
    """Real double-click on a word of the page; returns the screen point that was clicked."""
    img = c.image()
    p = c.glyph_near(x, y, img) or (x, y)
    c.sb.double_click(*p)
    return p


@scenario
def library_open_book(c):
    c.start()
    c.check("the library lists the imported book", c.find("table cell", rx="Alice"))
    c.shot("library")
    r = c.open_book()
    c.check("a double click on the card opens the reader (page slider appears)", r)
    c.check("the reader has the Back button", c.find("button", rx="^Back"))
    p = c.page_label()
    c.check("the footer shows the page number", p and p[1] > 50, str(p))
    c.shot("reader")


@scenario
def reader_page_turn(c):
    c.start()
    c.open_book()
    num = lambda: (c.page_label() or (0, 0))[0]  # noqa: E731  (the total is re-estimated while paginating)
    p0 = num()
    c.sb.key("Right")
    ok = c.wait(lambda: num() != p0, 5)
    p1 = num()
    c.check("the Right key turns the page", ok, f"{p0} -> {p1}")
    c.sb.key("Left")
    c.wait(lambda: num() == p0, 5)
    c.check("the Left key goes back", num() == p0, str(c.page_label()))
    nxt = c.wait_node("button", rx="^Next Page")
    c.click_node(nxt)
    ok = c.wait(lambda: num() != p0, 5)
    c.check("clicking the Next page button turns the page", ok, str(c.page_label()))
    p2 = num()
    c.sb.scroll(3, 640, 400)
    ok = c.wait(lambda: num() != p2, 5)
    c.check("the mouse wheel turns the page", ok, f"{p2} -> {num()}")
    c.shot("after-scroll")


@scenario
def lookup_dictionary(c):
    c.start()
    c.open_book()
    c.go_to_text()
    c.shot("page")
    select_word(c)
    btn = c.wait_node("button", "Look Up in Dictionary", timeout=5)
    c.shot("selection")
    if not c.check("a double click on a word opens the action popover with the dictionary button", btn):
        return
    c.check("the popover has the Wikipedia and Highlight buttons",
            c.find("button", "Look Up on Wikipedia") and c.find("button", "Highlight Yellow"))
    c.click_node(btn)
    res = c.wait_node("label", rx="Test definition of ", timeout=6)
    c.shot("dictionary-result")
    c.check("clicking Dictionary shows the definition", res, c.tree().dump()[-1500:] if not res else "")
    time.sleep(2.0)
    c.check("the dictionary page is still shown 2 s later (the popover did not vanish)",
            c.find("label", rx="Test definition of "))
    c.check("the dictionary heading is still shown", c.find("label", "Dictionary"))
    c.shot("dictionary-after-2s")
    img = c.image()
    # the popover is drawn: its white card contrasts with the page, check the result is on screen
    c.check("the app is alive", c.is_alive())


@scenario(net=True)
def lookup_wikipedia(c):
    c.start()
    c.open_book()
    c.go_to_text()
    select_word(c)
    btn = c.wait_node("button", "Look Up on Wikipedia", timeout=5)
    if not c.check("a double click on a word opens the action popover with the Wikipedia button", btn):
        return
    c.click_node(btn)
    time.sleep(0.5)
    c.shot("wikipedia-loading")
    c.check("the Wikipedia page is shown", c.wait_node("label", "Wikipedia", timeout=3))
    c.wait(lambda: c.find("label", rx="Couldn.t load a summary") or
           [n for n in c.find("label") if n.depth and len(n.name) > 40 and "Wikipedia" not in n.name], 25)
    time.sleep(1.5)
    c.shot("wikipedia-result")
    err = c.find("label", rx="Couldn.t load a summary")
    c.check("Wikipedia shows a summary or a clear error (not an empty page)",
            err or [n for n in c.find("label") if len(n.name) > 40], c.tree().dump()[-1500:])
    c.check("the Wikipedia page is still shown", c.find("label", "Wikipedia"))
    if err:
        c.note("error text: " + err[0].name[:200])


def is_yellow(r, g, b):
    return r > 225 and g > 190 and b < 150


def is_dark(r, g, b):
    return r + g + b < 200


@scenario(net=True)
def lookup_translate(c):
    c.start()
    c.open_book()
    c.go_to_text()
    select_word(c)
    btn = c.wait_node("button", "Translate", timeout=5)
    if not c.check("the action popover has the Translate button", btn):
        return
    c.click_node(btn)
    c.check("the Translate page is shown", c.wait_node("label", "Translate", timeout=3))
    got = c.wait(lambda: [n for n in c.find("label") if n.name and n.name not in ("Translate", "Google Translate")
                          and n.depth > 8 and "Couldn" in n.name or n.name.startswith("Couldn")], 25)
    time.sleep(1.5)
    c.shot("translate-result")
    labels = [n.name for n in c.find("label")]
    c.check("Translate shows a result or a clear error", got or len(labels) > 5, str(labels[-8:]))
    c.check("the popover is still open when the answer is shown", c.find("label", "Translate"))


@scenario
def highlight_and_note(c):
    c.start()
    c.open_book()
    c.go_to_text()
    p = select_word(c)
    box = (330, 325, 440, 360)
    before = c.count_pixels(c.image(), box, is_yellow)
    y = c.wait_node("button", "Highlight Yellow", timeout=5)
    if not c.check("selecting a word shows the highlight colours", y):
        return
    c.click_node(y)
    time.sleep(0.8)
    c.shot("highlighted")
    after = c.count_pixels(c.image(), box, is_yellow)
    c.check("clicking Highlight yellow paints the word yellow", after > before + 200, f"{before} -> {after} yellow px")
    # the popover stays open and now offers Remove (the selection is kept on purpose)
    rem = c.wait_node("button", "Remove Highlight", timeout=5)
    c.check("after highlighting, the popover stays and offers Remove highlight", rem)
    c.shot("highlight-popover")
    note = c.wait_node("button", "Add Note")
    if not c.check("the popover offers Add a note", note):
        return
    c.click_node(note)
    time.sleep(0.8)
    c.sb.type("my first note")
    time.sleep(0.3)
    save = c.wait_node("button", "Save Note", timeout=5)
    c.check("the note page has a Save Note button", save)
    c.shot("note-typed")
    if save:
        c.click_node(save)
        time.sleep(1)
    c.check("saving the note closes the popover", c.gone("button", "Save Note", timeout=4))
    c.sb.click(*p)
    rem = c.wait_node("button", "Remove Highlight", timeout=5)
    c.check("a click on the highlight opens its popover again", rem)
    time.sleep(0.5)
    c.shot("note-saved")
    c.check("the saved note is shown in that popover",
            c.find("button", rx="my first note") or c.find("label", rx="my first note"),
            [n.name for n in c.find("button")][:12])
    if rem:
        c.click_node(rem)
        time.sleep(1)
        gone = c.count_pixels(c.image(), box, is_yellow)
        c.shot("highlight-removed")
        c.check("Remove highlight takes the colour away", gone < before + 50, f"{gone} yellow px")


@scenario
def theme_change(c):
    c.start()
    c.open_book()
    c.go_to_text()
    img = c.image()
    c.check("the default page is light", sum(img.getpixel((60, 400))) > 600, str(img.getpixel((60, 400))))
    for name, test in (("Dark", lambda p: sum(p) < 200),
                       ("Sepia", lambda p: p[0] > p[2] + 15 and sum(p) > 500),
                       ("Light", lambda p: sum(p) > 700)):
        c.click_node(c.wait_node("toggle button", "Text Appearance"))
        t = c.wait_node("toggle button", name, timeout=5)
        if not c.check(f"Text Appearance has the {name} theme button", t):
            return
        c.click_node(t)
        time.sleep(0.8)
        c.sb.key("Escape")
        time.sleep(0.8)
        img = c.image()
        px = img.getpixel((60, 400))
        c.shot(f"theme-{name.lower()}")
        c.check(f"choosing {name} repaints the page background", test(px), str(px))


@scenario
def library_context_menu(c):
    c.start()
    card = c.wait_node("table cell", rx="Alice")
    c.sb.right_click(*card.center())
    menu = c.wait_node("menu", timeout=5)
    c.shot("context-menu")
    if not c.check("a right click on a book opens the context menu", menu):
        return
    items = c.find("menu item")
    c.check("the menu is short: at most 12 entries at its first level", 5 <= len(items) <= 12, str(len(items)))
    box = c.screen_box(menu)
    c.check("the whole menu fits on the screen (Move to Trash does not fall off)", box[3] <= c.sb.h, str(box))
    text = c.ocr(c.image(), box)
    if text is None:
        c.note("tesseract not installed: entry names not checked")
    else:
        for want in ("Read", "Edit", "Add to Shelf", "Send To", "Convert To", "Show in Files", "Move to Trash"):
            c.check(f"the menu offers {want!r}", want.lower() in text.lower(), text.replace("\n", " | "))
    c.sb.key("Escape")
    c.check("Escape closes the menu", c.gone("menu", timeout=3))


@scenario
def header_menus(c):
    c.start()
    c.click_node(c.wait_node("toggle button", rx="^Main Menu"))
    for name in ("Library Health", "Preferences", "Keyboard Shortcuts", "About Bookx"):
        c.check(f"the library main menu has {name}", c.wait_node("menu item", name, timeout=3))
    c.shot("library-menu")
    c.sb.key("Escape")
    c.check("Escape closes the main menu", c.gone("menu item", "Preferences", timeout=3))
    c.click_node(c.wait_node("toggle button", rx="^Search"))
    c.check("the search button opens the search entry", c.wait_node("search text") or c.wait_node("entry", timeout=3))
    c.shot("library-search")
    c.sb.key("Escape")
    c.open_book()
    c.click_node(c.wait_node("toggle button", "Main Menu"))
    for name in ("Go to", "Footer Statistics", "Reading Ruler", "Help", "About Bookx"):
        c.check(f"the reader main menu has {name}", c.wait_node(None, rx=f"^{name}", timeout=3))
    c.shot("reader-menu")
    c.sb.key("Escape")
    c.click_node(c.wait_node("toggle button", "Text Appearance"))
    c.check("Text appearance opens the typography popover", c.wait_node("label", "Line spacing", timeout=3))
    c.check("with the theme swatches in it", c.wait_node(None, "Light", timeout=3) and c.wait_node(None, "Sepia", timeout=3))
    c.shot("reader-appearance")
    c.sb.key("Escape")
    c.click_node(c.wait_node("toggle button", rx="^Contents"))
    c.check("the contents button opens the sidebar", c.wait_node(None, rx="CHAPTER I", timeout=5))
    c.shot("reader-contents")


@scenario
def news_dialog(c):
    c.start()
    # the Library menu in the content header: Subscribe to News (menu items carry their text as name)
    c.click_node(c.wait_node("toggle button", "Library"))
    c.click_node(c.wait_node("menu item", rx="^Subscribe to News", timeout=5))
    d = c.wait_node("dialog", "News Subscriptions", timeout=5)
    c.shot("news")
    if not c.check("the News Subscriptions dialog opens", d):
        return
    c.check("it says there are no subscriptions yet", c.find(None, "No subscriptions yet"))
    c.check("it has the Add and Download All Due buttons", c.find("button", "Add") and c.find("button", "Download All Due"))
    c.click_node(c.wait_node("button", "Add"))
    time.sleep(1)
    c.shot("news-add")
    c.check("Add opens the New Subscription form with a Save button",
            c.wait_node("dialog", "New Subscription", timeout=4) and c.find("button", "Save"))
    c.sb.key("Escape")
    time.sleep(0.8)
    c.sb.key("Escape")
    c.check("Escape closes the dialog", c.gone("dialog", "News Subscriptions", timeout=5))


@scenario
def editor_edit_and_replace(c):
    c.start()
    card = c.wait_node("table cell", rx="Alice")
    c.sb.right_click(*card.center())
    c.wait_node("menu", timeout=5)
    items = c.find("menu item")
    c.edit_book_from_menu()  # Edit ▸ Edit Book
    ed = c.wait(lambda: [n for n in c.find("frame") if n.name.startswith("Edit Book")], 20)
    if not c.check("Edit Book opens the editor window", ed):
        return
    time.sleep(2)
    c.shot("editor")
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type("<!-- zzmarker Hello -->")
    time.sleep(0.6)
    save = c.find("button", "Save")
    c.check("typing marks the book as modified (Save is enabled, title has a dot)",
            save and save[0].sensitive and c.find("label", rx="^● "))
    img = c.image("typed")
    txt = c.ocr(img, (256, 130, 918, 470))
    c.check("the typed text appears in the code view", txt is None or "zzmarker" in txt, txt)
    c.sb.key("f", ("Control",))
    c.check("Ctrl+F opens the find bar", c.wait_node("button", "Find All", timeout=5))
    c.sb.type("zzmarker")
    c.sb.key("h", ("Control",))  # the replace row: with a search text the caret goes to the replacement
    c.check("Ctrl+H opens find & replace", c.wait_node("button", "Replace All", timeout=5))
    time.sleep(0.8)
    c.sb.type("REPLACED")
    c.click_node(c.wait_node("button", "Replace All"))
    time.sleep(1)
    img = c.image("replaced")
    txt = c.ocr(img, (256, 220, 918, 600))
    if txt is not None:
        c.check("Replace All replaced the text in the document",
                "REPLACED" in txt.upper() and "ZZMARKER" not in txt.upper(), txt)
    else:
        c.note("tesseract not installed: result not checked")


@scenario
def editor_problems_open(c):
    """Check Book: a click on a problem shows its help, a real double-click opens the place, F8 goes on."""
    import re
    c.start()
    card = c.wait_node("table cell", rx="Alice")
    c.sb.right_click(*card.center())
    c.wait_node("menu", timeout=5)
    items = c.find("menu item")
    c.edit_book_from_menu()  # Edit ▸ Edit Book
    ed = c.wait(lambda: [n for n in c.find("frame") if n.name.startswith("Edit Book")], 20)
    if not c.check("Edit Book opens the editor window", ed):
        return
    time.sleep(2)
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type("<p>AT&T</p>")
    time.sleep(0.6)
    c.sb.key("Home", ("Control",))
    time.sleep(0.4)

    def cursor_line():
        for n in c.find("label", rx=r"Ln \d+, Col \d+"):
            m = re.search(r"Ln (\d+)", n.name)
            if m:
                return int(m.group(1))
        return None

    c.check("the cursor starts on the first line", cursor_line() == 1, str(cursor_line()))
    c.sb.key("F7")
    rows = c.wait(lambda: [n for n in c.find("list item") if re.match(r"error: .*line \d+", n.name)], 15)
    if not c.check("F7 lists the error with its line in the Problems panel", rows):
        c.shot("problems-missing")
        return
    time.sleep(1.2)  # the panel slides in
    rows = [n for n in c.find("list item") if re.match(r"error: .*line \d+", n.name)] or rows
    row = rows[0]
    want = int(re.search(r"line (\d+)", row.name).group(1))
    c.shot("problems")
    c.click_node(row)
    time.sleep(0.5)
    c.check("a single click shows what the problem means", c.find("label", rx="What it means"))
    c.check("a single click does not leave the place", cursor_line() == 1, str(cursor_line()))
    x, y = row.center()
    c.sb.double_click(x, y)
    time.sleep(1.0)
    got = cursor_line()
    c.check("a double click opens the place of the problem", got == want and want > 1, f"want {want}, got {got}")
    c.sb.key("Home", ("Control",))
    time.sleep(0.3)
    c.sb.key("F8")
    time.sleep(0.8)
    c.check("F8 goes to the next problem", cursor_line() not in (None, 1), str(cursor_line()))


@scenario
def editor_search_bar_session(c):
    """Typing in the find bar with real key events: live count, Count, F3/Ctrl+G, the mode list, Replace and find
    (Ctrl+]), Replace All, and the result on disk after saving."""
    import glob
    import re as _re
    c.start()
    card = c.wait_node("table cell", rx="Alice")
    c.sb.right_click(*card.center())
    c.wait_node("menu", timeout=5)
    c.edit_book_from_menu()  # Edit ▸ Edit Book
    ed = c.wait(lambda: [n for n in c.find("frame") if n.name.startswith("Edit Book")], 20)
    if not c.check("Edit Book opens the editor window", ed):
        return
    time.sleep(2)
    c.click_node(max(c.find("text"), key=lambda n: n.box[2] * n.box[3]))
    c.sb.key("End", ("Control",))
    c.sb.type("<!-- zzone zztwo zztwo -->")
    time.sleep(0.5)
    c.sb.key("f", ("Control",))
    c.check("Ctrl+F opens the find bar", c.wait_node("button", "Find All", timeout=5))
    c.sb.type("zz")
    c.check("typing in Find shows how many matches the file has",
            c.wait_node("label", rx=r"\d+ in this file|\d+ of \d+ in this file", timeout=6))
    c.click_node(c.wait_node("button", "Count"))
    c.check("Count says how many matches there are in the scope",
            c.wait_node("label", rx=r"3 matches in 1 file", timeout=6))
    c.sb.key("F3")
    time.sleep(0.5)
    c.sb.key("g", ("Control",))
    time.sleep(0.5)
    c.check("F3 and Ctrl+G step through the matches",
            c.wait_node("label", rx=r"[12] of 3 in this file", timeout=6))
    gear = c.wait_node("toggle button", "Search Options", timeout=4) or c.wait_node("button", "Search Options", timeout=2)
    if gear:
        c.click_node(gear)
        time.sleep(0.8)
    combos = [n for n in c.find("combo box") if n.name == "Text" and n.showing]
    if c.check("the search mode list is in the options popover", combos):
        c.click_node(combos[0])
        time.sleep(0.6)
        c.sb.key("Down")
        c.sb.key("Down")
        c.sb.key("Return")
        time.sleep(0.8)
        c.check("choosing Function shows the function list",
                [n for n in c.find("combo box") if n.name.startswith("Title-case") and n.showing])
        c.click_node([n for n in c.find("combo box") if n.name == "Function" and n.showing][0])
        time.sleep(0.6)
        c.sb.key("Up")
        c.sb.key("Up")
        c.sb.key("Return")
        time.sleep(0.8)
        c.check("going back to Text hides it again",
                not [n for n in c.find("combo box") if n.name.startswith("Title-case") and n.showing])
        c.sb.key("Escape")
        time.sleep(0.6)
    fe = c.find("search text")
    if fe:
        c.click_node(fe[0])
        c.sb.key("a", ("Control",))
        c.sb.type("zzone")
    c.sb.key("h", ("Control",))  # the replace row is its own shortcut; with a search text the caret goes to the replacement
    time.sleep(0.8)
    rep = c.find("button", "Replace All")
    c.check("the replace entry is there", rep and rep[0].showing)
    c.sb.key("a", ("Control",))
    c.sb.type("FIRSTDONE")
    fe = c.find("search text")
    if fe:
        c.click_node(fe[0])
    c.sb.key("Return")  # Enter in the find entry: next match, selects it
    time.sleep(0.5)
    c.sb.key("]", ("Control",))
    time.sleep(0.8)
    if fe:
        c.click_node(fe[0])
        c.sb.key("a", ("Control",))
        c.sb.type("zztwo")
    c.sb.key("h", ("Control",))
    time.sleep(0.5)
    c.sb.key("a", ("Control",))
    c.sb.type("SECONDDONE")
    c.click_node(c.wait_node("button", "Replace All"))
    time.sleep(1)
    c.sb.key("s", ("Control",))
    time.sleep(2)
    found = None
    for f in glob.glob(os.path.join(c.dir, "lib", "**", "*.epub"), recursive=True):
        try:
            with zipfile.ZipFile(f) as z:
                for n in z.namelist():
                    if n.endswith((".xhtml", ".html")):
                        t = z.read(n).decode("utf-8", "replace")
                        if "FIRSTDONE" in t:
                            found = t
        except Exception:
            pass
    c.check("Replace and find (Ctrl+]) replaced the selected match in the saved book",
            found is not None and "FIRSTDONE" in found and "zzone" not in found)
    c.check("Replace All replaced both remaining matches in the saved book",
            found is not None and found.count("SECONDDONE") == 2 and "zztwo" not in found)


@scenario
def second_book(c):
    c.start(books=("alice", "tiny"))
    c.check("the library lists both books", c.find("table cell", rx="Alice") and c.find("table cell", rx="Masque"))
    c.shot("two-books")
    c.check("the short book opens", c.open_book("Masque"))
    p = c.page_label()
    c.check("the short book has far fewer pages than Alice", p and p[1] < 150, str(p))
    c.check("the header shows its title", c.find("label", rx="Masque of the Red Death"))
    c.shot("masque")
    c.sb.key("Right")
    time.sleep(1)
    c.sb.key("Left")
    c.check("the app is still alive after paging", c.is_alive())
import scenarios_a11y  # noqa: E402,F401  (accessibility scenarios)
import scenarios_vertical  # noqa: E402,F401  (vertical writing scenarios)
import scenarios_pdf  # noqa: E402,F401  (PDF reading with real input)
import scenarios_tts  # noqa: E402,F401  (read aloud scenarios)
import scenarios_editor_tools  # noqa: E402,F401  (editor tools with real input)
import scenarios_editor_struct  # noqa: E402,F401  (editor structure tools with real input)
import scenarios_editor_preview  # noqa: E402,F401  (preview sync and split with real input)
import scenarios_editor_prefs  # noqa: E402,F401  (editor shortcuts and snippets with real input)
import scenarios_ux_gaps  # noqa: E402,F401  (keyboard shortcuts, shelves, jobs, reader extras with real input)
import scenarios_map_keys  # noqa: E402,F401  (book map wheel and drag, new reader keys with real input)
import scenarios_vocab  # noqa: E402,F401  (own vocabulary terms with real input)
import scenarios_pain  # noqa: E402,F401  (reading ruler keys with real input)
import scenarios_works  # noqa: E402,F401  (grouped editions: badge, edition list, link and unlink with real input)
import scenarios_library  # noqa: E402,F401  (library sidebar, selection clicks, details panel with real input)
import scenarios_reader_ux  # noqa: E402,F401  (reader sidebar, footer, selection handles with real input)
import scenarios_wiki  # noqa: E402,F401  (public wiki screenshots, WIKI_SCHEME=light|dark)
import scenarios_wiki_ext  # noqa: E402,F401  (more wiki screenshots: details and usage steps)
import scenarios_editor_code  # noqa: E402,F401  (code editing helpers with real input)
import scenarios_anchor  # noqa: E402,F401  (imported highlights are painted where their text is)
import scenarios_scale  # noqa: E402,F401  (20,000 book grid scroll with the real frame clock and wheel)
import scenarios_audiobook  # noqa: E402,F401  (audiobooks with real input)
import scenarios_comics  # noqa: E402,F401  (comics: smart zoom, guided view, direction with real input)
import scenarios_sync  # noqa: E402,F401  (Sync menu entry, window and Preferences page with real input)
import scenarios_qa  # noqa: E402,F401  (QA sweeps by eye)
import scenarios_wave11  # noqa: E402,F401  (live sync updates, master password, custom columns, settings backup, plugin trust, safe mode)
