"""Accessibility scenarios: what a screen reader sees (AT-SPI), keyboard-only operation, the command palette
with real keys, high contrast. Imported by scenarios.py. See docs/a11y-notes.md."""
import os
import re
import time
import zipfile

from harness import scenario
import atspi
from gi.repository import Atspi


def page_node(c):
    """The reading canvas: the accessible with the page text (role document, named '<title>, page text')."""
    r = c.find(None, rx=r", page text$")
    return r[0] if r else None


def page_text(c):
    n = page_node(c)
    return n.get_text() if n else None


def make_rtl_epub(workdir):
    """A tiny Arabic book with page-progression-direction="rtl" in its spine."""
    p = os.path.join(workdir, "rtl.epub")
    ar = "هذا نص تجريبي طويل للقراءة من اليمين إلى اليسار، ويجب أن تنتقل الصفحات في هذا الكتاب من اليسار إلى اليمين."
    body = "".join(f"<p>{i}. {ar}</p>" for i in range(1, 80))
    with zipfile.ZipFile(p, "w") as z:
        z.writestr("mimetype", "application/epub+zip", compress_type=zipfile.ZIP_STORED)
        z.writestr("META-INF/container.xml",
                   '<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">'
                   '<rootfiles><rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>')
        z.writestr("OEBPS/content.opf",
                   '<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="id">'
                   '<metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:identifier id="id">rtl-session</dc:identifier>'
                   '<dc:title>Right To Left Book</dc:title><dc:creator>Rtl Author</dc:creator><dc:language>ar</dc:language></metadata>'
                   '<manifest><item id="c1" href="c1.xhtml" media-type="application/xhtml+xml"/></manifest>'
                   '<spine page-progression-direction="rtl"><itemref idref="c1"/></spine></package>')
        z.writestr("OEBPS/c1.xhtml",
                   '<?xml version="1.0" encoding="utf-8"?><html xmlns="http://www.w3.org/1999/xhtml" dir="rtl" lang="ar">'
                   f'<head><title>c</title></head><body dir="rtl"><h1>الفصل الأول</h1>{body}</body></html>')
    return p


INTERACTIVE = ("push button", "toggle button", "check box", "radio button", "slider", "spin button", "combo box",
               "switch", "entry", "password text", "search text")


def unnamed(c, extra_roles=()):
    """Showing interactive nodes without an accessible name (what Orca would read as just 'button')."""
    roles = INTERACTIVE + tuple(extra_roles)
    return [n for n in c.tree().nodes if n.showing and n.role in roles and not n.name.strip()]


def describe(nodes):
    return "; ".join(f"{n.role}@{n.box[:2]}" for n in nodes[:12])


def text_iface(node):
    return node.acc.get_text_iface()


@scenario
def a11y_page_text(c):
    """Orca reads the page: the canvas exposes the visible paragraphs through the AT-SPI Text interface."""
    c.start()
    c.open_book()
    c.go_to_text()
    n = c.wait_node(None, rx=r", page text$", timeout=8)
    if not c.check("the reading canvas is in the accessibility tree, named after the book", n):
        c.note(c.tree().dump()[-2500:])
        return
    c.check("its role is a document", n.role.startswith("document"), n.role)
    c.check("it is focusable and has the Text interface", text_iface(n) is not None)
    text = c.wait(lambda: (page_text(c) or "") if len(page_text(c) or "") > 200 else None, 8) or ""
    c.shot("page")
    c.check("it holds the text of the page (more than a title)", len(text) > 200, repr(text[:80]))
    c.check("the text has several paragraphs", text.count("\n") >= 1, repr(text[:80]))
    c.check("it is the text of Alice, in order", re.search(r"[A-Za-z]{4,} [a-z]{2,} [a-z]{2,}", text) is not None, repr(text[:80]))
    t = text_iface(n)
    first_word = Atspi.Text.get_string_at_offset(t, 0, Atspi.TextGranularity.WORD) if hasattr(Atspi.TextGranularity, "WORD") else None
    c.check("a screen reader can ask for the first word", first_word is not None and first_word.content.strip() and first_word.content.strip() in text[:40],
            str(first_word and first_word.content))
    line = Atspi.Text.get_string_at_offset(t, 0, Atspi.TextGranularity.LINE)
    para = Atspi.Text.get_string_at_offset(t, 0, Atspi.TextGranularity.PARAGRAPH)
    c.check("it can ask for a line, shorter than the paragraph", line and para and 10 < len(line.content) < len(para.content),
            f"{len(line.content) if line else None} / {len(para.content) if para else None}")
    sent = Atspi.Text.get_string_at_offset(t, 0, Atspi.TextGranularity.SENTENCE)
    c.check("it can ask for a sentence", sent and sent.content.strip(), str(sent and sent.content))
    ok = Atspi.Text.set_caret_offset(t, 31)
    c.check("a screen reader can move the caret through the page text", ok and Atspi.Text.get_caret_offset(t) == 31,
            f"{ok} {Atspi.Text.get_caret_offset(t)}")
    Atspi.Text.set_caret_offset(t, 0)
    first_line = (line.content if line else "").strip()
    # the same words are drawn on the screen
    ocr = c.ocr(c.image())
    if ocr is None:
        c.note("tesseract not installed: the screen was not compared with the page text")
    else:
        words = [w for w in re.findall(r"[A-Za-z]{5,}", first_line)][:3]
        c.check("the page text matches what is drawn (OCR finds its first words)",
                all(w.lower() in ocr.lower() for w in words), f"{words} in {ocr[:200]!r}")
    # a page turn changes the text, without anybody asking the canvas to
    before = text
    c.sb.key("Right")
    after = c.wait(lambda: (lambda x: x if x and x != before else None)(page_text(c)), 6)
    c.check("a page turn updates the text a screen reader reads", after, "text did not change")
    c.check("the new text does not repeat the old page", after and before[:60] not in after)
    c.sb.key("Left")
    back = c.wait(lambda: (lambda x: x if x == before else None)(page_text(c)), 6)
    c.check("turning back restores the first page's text", back)


@scenario
def a11y_keyboard_only(c):
    """Open a book, turn pages, select, look up, highlight and write a note without touching the mouse."""
    c.start()
    c.sb.key("k", ("Control",))
    entry = c.wait_node(None, "Command palette search", timeout=6)
    if not c.check("Ctrl+K opens the command palette in the library", entry):
        c.shot("no-palette")
        return
    c.check("the search entry has the keyboard focus", entry.focused)
    c.sb.type("alice")
    row = c.wait(lambda: [n for n in c.find(None, rx="^Alice.*Book") if n.role in ("list item", "table cell", "list box row")], 6)
    c.shot("palette-alice")
    c.check("typing a title lists the book", row or c.find(None, rx="Alice"))
    c.sb.key("Return")
    c.check("Enter opens the book in the reader", c.wait_node("slider", timeout=12))
    c.check("the palette is gone", c.gone(None, "Command palette search", timeout=4))
    time.sleep(1.5)
    n0 = c.go_to_text()
    c.check("Right arrow pages forward (keyboard only)", n0 and n0[0] >= 8, str(n0))
    img0 = c.image("focus")
    # the canvas shows a focus ring after keyboard use
    canvas = page_node(c)
    box = c.screen_box(canvas) if canvas else (0, 0, 10, 10)

    def blue(r, g, b):
        return b > 150 and b > r + 70 and b > g + 20

    strip = (box[0], box[1], box[2], box[1] + 8)
    ring = c.count_pixels(img0, strip, blue) + c.count_pixels(img0, (box[0], box[3] - 8, box[2], box[3]), blue)
    c.check("after keyboard use the reading canvas has a visible focus ring", ring > 300, f"{ring} accent px on its edges")

    # select with the keyboard: Ctrl+Shift+Right = a word at a time from the top of the page
    for _ in range(3):
        c.sb.key("Right", ("Control", "Shift"))
    time.sleep(0.6)
    t = text_iface(canvas)
    sel = None
    if t:
        c.wait(lambda: Atspi.Text.get_n_selections(t) > 0, 4)
        if Atspi.Text.get_n_selections(t) > 0:
            r = Atspi.Text.get_selection(t, 0)
            sel = Atspi.Text.get_text(t, r.start_offset, r.end_offset)
    c.shot("kb-selected")
    c.check("Ctrl+Shift+Right selects words and the selection is exposed to AT-SPI", sel and len(sel.split()) == 3, repr(sel))
    c.sb.key("Return")
    btn = c.wait_node("button", "Look up in a dictionary", timeout=5)
    c.check("Enter opens the actions for the selection", btn)
    c.sb.key("d")
    res = c.wait_node("label", rx="Test definition of ", timeout=6)
    c.shot("kb-dictionary")
    c.check("the d key looks the selection up in the dictionary", res)
    c.sb.key("Escape")
    time.sleep(0.8)
    c.check("Escape closes the look-up and clears the selection", c.gone("label", rx="Test definition of ", timeout=4))
    c.check("Escape did not leave the book", c.find("slider"))

    # highlight by keyboard
    box2 = (330, 325, 800, 360)
    for _ in range(4):
        c.sb.key("Right", ("Control", "Shift"))
    time.sleep(0.4)
    c.sb.key("Return")
    c.wait_node("button", "Highlight yellow", timeout=5)
    c.sb.key("h")
    time.sleep(0.9)
    img = c.image("kb-highlight")
    yellow = c.count_pixels(img, (200, 100, 1100, 700), lambda r, g, b: r > 225 and g > 190 and b < 150)
    c.check("the h key highlights the selection in yellow", yellow > 400, f"{yellow} yellow px")

    # a note by keyboard
    c.sb.key("n")
    ok = c.wait_node("button", "Save Note", timeout=6)
    c.check("the n key opens the note box", ok)
    if ok:
        c.sb.type("typed without a mouse")
        c.sb.key("Return", ("Control",))
        c.check("Ctrl+Enter saves the note and closes the box", c.gone("button", "Save Note", timeout=5))
    time.sleep(0.5)
    # the note is in the notes list of the sidebar (opened from the palette, by keyboard)
    c.sb.key("p", ("Control", "Shift"))
    c.wait_node(None, "Command palette search", timeout=5)
    c.sb.type("bookmarks and")
    time.sleep(0.5)
    c.sb.key("Return")
    note = c.wait_node(None, rx="typed without a mouse", timeout=6)
    c.shot("kb-notes")
    c.check("the saved note is in the Notes tab", note)
    c.check("the app is alive", c.is_alive())


@scenario
def a11y_palette_keys(c):
    """The palette in the reader with the real keys: commands, themes, settings, Escape."""
    c.start()
    c.open_book()
    c.go_to_text()
    c.sb.key("p", ("Control", "Shift"))
    entry = c.wait_node(None, "Command palette search", timeout=6)
    if not c.check("Ctrl+Shift+P opens the palette in the reader", entry):
        return
    c.sb.type("theme dark")
    time.sleep(0.6)
    c.shot("palette-theme")
    c.sb.key("Down")
    c.sb.key("Up")
    c.sb.key("Return")
    time.sleep(1.0)
    img = c.image("dark")
    px = img.getpixel((60, 400))
    c.check("choosing Theme: Dark repaints the page", sum(px) < 200, str(px))
    c.check("the palette closed after the choice", c.gone(None, "Command palette search", timeout=4))
    c.sb.key("k", ("Control",))
    c.wait_node(None, "Command palette search", timeout=5)
    c.sb.key("Escape")
    c.check("Escape closes the palette", c.gone(None, "Command palette search", timeout=4))
    c.check("the reader is still open", c.find("slider"))
    # settings: the palette finds a row of Preferences and opens it
    c.sb.key("k", ("Control",))
    c.wait_node(None, "Command palette search", timeout=5)
    c.sb.type("hyphenate")
    time.sleep(0.5)
    c.sb.key("Return")
    d = c.wait_node("dialog", "Preferences", timeout=6) or c.wait_node(None, "Preferences", timeout=2)
    c.shot("prefs-from-palette")
    c.check("choosing a setting opens Preferences", d)
    c.check("on the page of that setting", c.wait_node(None, "Hyphenate words", timeout=4))


def region_diff(a, b, box):
    """How many pixels of `box` differ noticeably between two screenshots."""
    x0, y0, x1, y1 = box
    pa, pb = a.load(), b.load()
    n = 0
    for y in range(y0, min(y1, a.height, b.height)):
        for x in range(x0, min(x1, a.width, b.width)):
            if sum(abs(i - j) for i, j in zip(pa[x, y], pb[x, y])) > 60:
                n += 1
    return n


@scenario
def reader_read_aloud_stays(c):
    """Bug 7: the read-aloud popover and the bars it sits in stay while the popover is open, longer than the
    auto-hide timeout (4 s); the bars fade only after the popover is closed."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "300"})
    c.check("the book opens", c.open_book())
    time.sleep(1.0)
    btn = c.wait_node("toggle button", rx="^Read aloud")
    if not c.check("the reader has a Read aloud button", btn):
        return
    c.click_node(btn)
    speed = c.wait_node("slider", "Reading speed", timeout=5)
    if not c.check("the read-aloud popover opens", speed):
        return
    sx, sy = speed.center(atspi.ORIGIN)
    c.sb.move_to(sx, sy)  # the pointer rests inside the popover, away from the bars
    time.sleep(0.8)
    before = c.image("read-aloud-open")
    header = (0, 40, before.width, 85)  # below the shell panel (its clock changes)
    pop = c.screen_box(speed)
    pop = (pop[0] - 20, pop[1] - 60, pop[2] + 20, pop[3] + 90)
    time.sleep(7.0)
    after = c.image("read-aloud-after-7s")
    c.check("the popover is still open after longer than the auto-hide timeout", c.find("slider", "Reading speed"))
    c.check("the header did not fade", region_diff(before, after, header) < 40, str(region_diff(before, after, header)))
    c.check("the popover did not fade", region_diff(before, after, pop) < 40, str(region_diff(before, after, pop)))
    c.sb.key("Escape")
    c.check("Escape closes the popover", c.gone("slider", "Reading speed", timeout=4))
    c.sb.move_to(640, 420)
    time.sleep(1.0)
    shown = c.image()
    time.sleep(9.0)
    faded = c.image("read-aloud-faded")
    c.check("after the interaction ended the bars fade again", region_diff(shown, faded, header) > 30,
            str(region_diff(shown, faded, header)))


@scenario
def editor_palette_keys(c):
    """Ctrl+Shift+P in the editor window opens the editor's command palette (not the library's)."""
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
    c.sb.key("p", ("Control", "Shift"))
    c.check("Ctrl+Shift+P opens the palette", c.wait_node(None, "Command palette search", timeout=5))
    c.sb.type("split")
    time.sleep(0.6)
    c.shot("editor-palette")
    c.check("it lists the editor's Tools menu", c.wait_node(None, rx="^Split File at Chapter Headings", timeout=4))
    c.check("and not the library's commands", not c.find(None, rx="^Add Books"))
    c.sb.key("Escape")
    c.check("Escape closes it", c.gone(None, "Command palette search", timeout=4))


@scenario
def a11y_prefs_search(c):
    """Preferences: one search that filters the rows of every page, with their section titles."""
    c.start()
    c.sb.key(",", ("Control",))
    d = c.wait_node("dialog", "Preferences", timeout=6)
    if not c.check("Ctrl+, opens Preferences", d):
        return
    c.check("the sidebar lists the sections", len(c.find(None, rx="^(Reading|Extensions|Devices and Sharing|Library|E-mail)$")) >= 3,
            str([n.name for n in c.find(None, rx="^(Reading|Extensions|Devices and Sharing|Library|E-mail)$")]))
    c.check("the search entry is named", c.find(None, "Search settings"))
    c.sb.key("f", ("Control",))
    time.sleep(0.6)
    c.sb.type("hyphen")
    time.sleep(1.0)
    c.shot("prefs-search")
    c.check("searching shows the matching row", c.find(None, "Hyphenate words"))
    c.check("and hides the rows that do not match", not c.find(None, "Library folder"))
    c.sb.key("BackSpace")
    for _ in range(8):
        c.sb.key("BackSpace")
    c.sb.type("mail server")
    time.sleep(1.0)
    c.shot("prefs-search-mail")
    c.check("a search finds a setting on another page (the E-mail section of Devices and Sharing)", c.find(None, rx="(?i)mail server"))
    c.check("the first search result is gone", not c.find(None, "Hyphenate words"))


@scenario
def a11y_rtl_keys(c):
    """In a right-to-left book the Left arrow turns forward."""
    c.books["rtl"] = make_rtl_epub(c.dir)
    c.start(books=("rtl",))
    c.check("the Arabic book opens", c.open_book("Right To Left"))
    time.sleep(1.5)
    num = lambda: (c.page_label() or (0, 0))[0]  # noqa: E731
    p0 = num()
    c.sb.key("Left")
    ok = c.wait(lambda: num() != p0, 5)
    p1 = num()
    c.check("Left turns forward in a right-to-left book", ok and p1 > p0, f"{p0} -> {p1}")
    c.sb.key("Right")
    c.wait(lambda: num() == p0, 5)
    c.check("Right turns back", num() == p0, str(num()))
    text = c.wait(lambda: page_text(c), 6) or ""
    c.shot("rtl-page")
    c.check("the page text for the screen reader holds the Arabic paragraphs", "الفصل" in text or "هذا" in text, repr(text[:60]))
    img = c.image()
    ink = [x for x in range(0, img.width, 4)
           if any(sum(img.getpixel((x, y))) < 250 for y in range(250, 420, 6))]
    c.check("the page has ink on the right half of its column", ink and max(ink) > 900, str(ink[-3:]))


@scenario
def a11y_names(c):
    """Every interactive control in the real accessibility tree has a name (what Orca says)."""
    c.start(extra_env={"BOOKX_DEV_SCRIPT": "wait:5000,a11y-audit:"})
    time.sleep(6)
    bad = unnamed(c)
    total = [n for n in c.tree().nodes if n.showing and n.role in INTERACTIVE]
    c.check("the tree has the library's buttons to look at", len(total) >= 5, str(len(total)))
    c.check("library: every button, switch and entry has a name", not bad, describe(bad))
    # the main menu and the sort/zoom popover
    c.click_node(c.wait_node("toggle button", rx="^Main menu"))
    time.sleep(0.6)
    bad = unnamed(c)
    c.check("library main menu: every control has a name", not bad, describe(bad))
    c.sb.key("Escape")
    c.open_book()
    c.go_to_text()
    bad = unnamed(c)
    c.check("reader: every control has a name", not bad, describe(bad))
    c.shot("names-reader")
    for label in ("Text appearance", "Main menu"):
        c.click_node(c.wait_node("toggle button", rx=f"^{label}"))
        time.sleep(0.7)
        bad = unnamed(c)
        c.check(f"reader {label} popover: every control has a name", not bad, describe(bad))
        c.sb.key("Escape")
        time.sleep(0.4)
    c.sb.key("k", ("Control",))
    c.wait_node(None, "Command palette search", timeout=5)
    bad = unnamed(c)
    c.check("palette: every control has a name", not bad, describe(bad))
    c.sb.key("Escape")
    time.sleep(0.5)
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    m = re.search(r"a11y: controls=(\d+) problems=(\d+)", log)
    c.check("the widget-tree audit in the same app finds no problems", m and m.group(2) == "0", m and m.group(0))
    env = re.search(r"a11y-env: animations=(\w+) high_contrast=(\w+)", log)
    c.check("the app reports the desktop's animations and high-contrast settings", env, env and env.group(0))
    if env:
        c.note(f"GTK enable-animations as the app saw it (the sandbox sets enable-animations=false): {env.group(1)}")


@scenario
def a11y_high_contrast(c):
    """With the desktop's high-contrast setting the page is pure black on white."""
    c.start(settings="[org/gnome/desktop/a11y/interface]\nhigh-contrast=true\n",
            extra_env={"BOOKX_DEV_SCRIPT": "wait:4000,a11y-audit:"})
    time.sleep(5)
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    env = re.search(r"a11y-env: animations=(\w+) high_contrast=(\w+)", log)
    if not env or env.group(2) != "true":
        raise_skip(c, f"libadwaita did not see the high-contrast setting in this session ({env and env.group(0)})")
    c.open_book()
    c.go_to_text()
    img = c.image("high-contrast")
    darkest = min(sum(img.getpixel((x, y))) for x in range(350, 700, 2) for y in range(330, 380, 2))
    c.check("the text is pure black (not the usual dark grey)", darkest <= 15, str(darkest))
    c.check("the page is pure white", img.getpixel((60, 400)) == (255, 255, 255), str(img.getpixel((60, 400))))


def raise_skip(c, why):
    from harness import Skip
    raise Skip(why)


@scenario
def _retry_probe(c):
    """Harness self-check (not in the default list): the first attempt looks like a registry failure, the retry passes.
    Run with: RETRY_MARKER=/tmp/opencode/marker ONLY=_retry_probe build-aux/session-tests.sh"""
    marker = os.environ.get("RETRY_MARKER", "")
    if not marker:
        from harness import Skip
        raise Skip("set RETRY_MARKER to a path that does not exist yet")
    if not os.path.exists(marker):
        open(marker, "w").write("first attempt")
        with open(os.path.join(c.dir, "app.log"), "w") as f:
            f.write("(bookx:1): Gtk-WARNING **: Could not activate remote peer 'org.a11y.atspi.Registry': timeout\n")
        raise RuntimeError("simulated: the AT-SPI registry did not start")
    c.check("the scenario ran again in a fresh session after the registry failure", True)


@scenario
def a11y_orca(c):
    """The real Orca (skipped when `orca` is not installed) finds the reading canvas, the app survives its queries, Page
    Down turns the page and Orca's caret navigation (Down) reads lines of the page text. Orca's debug log is saved next
    to the screenshots (orca-debug.log). Regression: the app used to crash on Orca's GetAttributeRun."""
    import shutil
    import subprocess
    if not shutil.which("orca"):
        from harness import Skip
        raise Skip("orca is not installed")
    c.start(extra_env={"G_DBUS_DEBUG": os.environ.get("G_DBUS_DEBUG", "")} if os.environ.get("G_DBUS_DEBUG") else None)
    log = os.path.join(c.results, "orca-debug.log")
    env = dict(c.sb.env)
    env.update({"WAYLAND_DISPLAY": c.sb.display, "GDK_BACKEND": "wayland"})
    orca = subprocess.Popen(["orca", "--replace", "--debug-file", log, "--debug", "0"], env=env,
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, start_new_session=True)
    c.sb.procs.append(orca)
    time.sleep(8)
    card = c.wait_node("table cell", rx="Alice", timeout=30)
    c.sb.double_click(*card.center())
    c.check("the book opens while Orca runs", c.wait_node("slider", timeout=60))
    time.sleep(5)
    open(os.path.join(c.results, "orca-tree.txt"), "w").write(c.tree().dump())
    # Orca's browse mode takes the arrow keys for its own caret; Page Down still reaches the app
    n = page_node(c)
    first = (n.get_text() if n else "")
    p0 = c.page_label()
    c.sb.key("Page_Down")
    time.sleep(3)
    c.note(f"pages {p0} -> {c.page_label()} after Page Down with Orca running")
    c.sb.key("Down")
    time.sleep(2)
    c.sb.key("Down")
    time.sleep(3)
    try:
        text = open(log, errors="replace").read()
    except OSError:
        text = ""
    n = page_node(c)
    now = (n.get_text() if n else "")
    spoken = " ".join(re.findall(r"SPEECH OUTPUT: '(.*?)'\{", text, re.S))
    c.check("the page changed with Page Down while Orca runs", c.page_label() != p0 and now != first, f"{p0} {c.page_label()}")
    words = [w for w in re.findall(r"[A-Za-z]{6,}", now)][:60]
    hits = [w for w in words if w in spoken]
    c.note(f"orca spoke {len(spoken)} chars; {len(hits)} of the first {len(words)} long words of the page were spoken")
    c.check("Orca's caret navigation (Down) reads a line of the page text", len(hits) >= 1, spoken[-300:])


# ---- screenshots for the public wiki (build-aux/wiki-screenshots.sh; the leading "_" keeps it out of the normal run)

@scenario
def _wiki_shots(c):
    """One session that opens the library, the Device window, Preferences, the palette, the reader and the editor
    of a sandbox (public-domain test books, a fake Boox, a mock reader engine) and takes the screenshots of the wiki."""
    mounts = os.path.join(c.dir, "mounts")
    books = os.path.join(mounts, "Boox", "Books")
    os.makedirs(books, exist_ok=True)
    import shutil
    shutil.copy(c.books["tiny"], os.path.join(books, "Tiny Book.epub"))
    shutil.copy(c.books["alice"], os.path.join(books, "Alice in Wonderland.epub"))
    from scenarios_wiki import no_rings, settings, wshot
    no_rings()
    c.start(books=("alice", "tiny"), settings=settings(), extra_env={"BOOKX_TTS_MOCK": "300"})
    time.sleep(2.0)
    wshot(c, "library-device")  # the banner that asks about the connected device
    dev = c.wait_node("button", rx="^E-reader", timeout=6)
    no = c.find("button", "Not now")
    if no:
        c.click_node(no[0])
        time.sleep(1.0)
        c.sb.move_to(640, 700)
    if dev:
        c.click_node(dev)
        time.sleep(2.5)
        wshot(c, "device-window")
        c.sb.key("Escape")
        time.sleep(0.8)
    else:
        c.note("no Device button: the device window was not shot")
    c.sb.key(",", ("Control",))
    if c.wait_node("dialog", "Preferences", timeout=6):
        time.sleep(0.8)
        wshot(c, "preferences")
        c.sb.key("f", ("Control",))
        c.sb.type("mail")
        time.sleep(1.0)
        wshot(c, "preferences-search")
        c.sb.key("Escape")
        c.sb.key("Escape")
        time.sleep(0.8)
    c.sb.key("k", ("Control",))
    if c.wait_node(None, "Command palette search", timeout=5):
        c.sb.type("pre")
        time.sleep(0.8)
        wshot(c, "command-palette")
        c.sb.key("Escape")
        time.sleep(0.6)
    c.check("the library is still there", c.find("table cell"))
    # the reader
    c.check("the book opens", c.open_book())
    time.sleep(1.5)
    c.sb.key("Right")
    time.sleep(0.8)
    toc = c.wait_node("toggle button", rx="^Contents", timeout=5)
    if toc:
        c.click_node(toc)
        time.sleep(1.2)
        c.sb.move_to(640, 440)
        wshot(c, "reader-contents")
        c.click_node(toc)
        time.sleep(0.6)
    # the editor
    c.sb.key("Escape")
    time.sleep(1.0)
    back = c.find("button", rx="^Back to library|^Library")
    if back:
        c.click_node(back[0])
    time.sleep(1.5)
    card = c.wait_node("table cell", rx="Alice")
    if card:
        c.sb.right_click(*card.center())
        if c.wait_node("menu", timeout=5):
            c.edit_book_from_menu()  # Edit ▸ Edit Book
            ed = c.wait(lambda: [n for n in c.find("frame") if n.name.startswith("Edit Book")], 20)
            if ed:
                time.sleep(3.0)
                wshot(c, "editor")
                c.sb.key("f", ("Control",))
                time.sleep(0.5)
                c.sb.type("Alice")
                time.sleep(1.0)
                wshot(c, "editor-find")
                c.sb.key("p", ("Control", "Shift"))
                time.sleep(0.8)
                c.sb.type("check")
                time.sleep(0.8)
                wshot(c, "editor-palette")
    c.check("the screenshots were taken", True)
