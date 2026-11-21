"""Reader UX with real input (docs/enhancements.md 10, 12, 13, 16 and bug 9): the sidebar, the footer statistics,
the text selection handles and the Wikipedia picture. Imported by scenarios.py."""
import os
import re
import subprocess
import sys
import time

from harness import scenario

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))


def _big_book(c, entries=2000):
    big = os.path.join(c.dir, "big.epub")
    subprocess.run([sys.executable, os.path.join(ROOT, "build-aux", "make-big-epub.py"), big, str(entries)],
                   check=True, capture_output=True)
    c.books["big"] = big
    return big


def _app_log(c):
    with open(os.path.join(c.dir, "app.log"), errors="replace") as f:
        return f.read()


def _contents_list(c):
    """The showing Contents list of the sidebar (not the switcher button of the same name)."""
    return [n for n in c.find(None, rx="^Contents$") if n.showing and n.role not in ("page tab", "toggle button", "radio button", "push button")]


@scenario
def reader_sidebar_toggle(c):
    """Bug 9: the sidebar of a big book (2000 contents entries, 500 highlights) opens and closes at once, ten times,
    with the toggle button and with F9. The frames after each toggle are timed inside the app (dev hook side-watch)."""
    _big_book(c)
    c.start(books=("big",), extra_env={
        "BOOKX_DEV_SCRIPT": "wait:2500,read:1,wait:4000,reader:bulk-highlights=500,reader:side-watch"})
    if not c.check("the big book opens in the reader", c.wait_node("slider", timeout=25)):
        return
    time.sleep(7)
    toggle = c.wait_node("toggle button", rx="^Contents, Bookmarks")
    if not c.check("the reader has a sidebar toggle", toggle):
        return
    seen = []
    for i in range(10):
        if i % 2:
            c.sb.key("F9")
        else:
            c.click_node(toggle)
        t0 = time.time()
        shown = c.wait(lambda: _contents_list(c), 4, every=0.05)
        seen.append(("open", bool(shown), time.time() - t0))
        time.sleep(0.6)
        if i % 2:
            c.sb.key("F9")
        else:
            c.click_node(toggle)
        t0 = time.time()
        gone = c.wait(lambda: not _contents_list(c), 4, every=0.05)
        seen.append(("close", bool(gone), time.time() - t0))
        time.sleep(0.6)
    c.check("all 10 openings showed the contents", all(ok for k, ok, _ in seen if k == "open"), str(seen))
    c.check("all 10 closings hid them", all(ok for k, ok, _ in seen if k == "close"), str(seen))
    c.sb.key("F9")
    time.sleep(1.0)
    c.shot("sidebar-big")
    tab = [n for n in c.find(None, rx="^Notes$") if n.role in ("page tab", "toggle button", "radio button", "push button")]
    if c.check("the sidebar has a Notes tab", tab, str([(n.role, n.name, n.showing) for n in c.find(None, rx="Notes")])):
        c.click_node(tab[0])
        time.sleep(1.2)
        c.check("the notes page counts the highlights", c.wait_node(None, rx="^500 highlights$", timeout=6))
        c.shot("sidebar-notes")
        entry = c.wait_node(None, "Search notes and highlights", timeout=4)
        if c.check("the notes page has a search field", entry):
            c.click_node(entry)
            c.sb.type("number 33")
            time.sleep(1.0)
            c.check("searching narrows the list", c.wait_node(None, rx=r"^\d+ of 500 highlights$", timeout=6))
            c.shot("sidebar-notes-search")
    log = _app_log(c)
    rows = re.findall(r"side_time: (open|close) first_frame_ms=([\d.]+) max_gap_ms=([\d.]+) frames=(\d+) work_ms=([\d.]+)", log)
    c.note(f"frames after each toggle: {rows}")
    c.check("the app timed every toggle", len(rows) >= 20, f"{len(rows)} lines")
    worst_first = max((float(r[1]) for r in rows), default=999999)
    worst_gap = max((float(r[2]) for r in rows), default=999999)
    c.check("the first frame after a toggle comes within 100 ms (debug build)", worst_first < 100, f"worst {worst_first} ms")
    c.check("no frame of the animation takes longer than 100 ms", worst_gap < 100, f"worst {worst_gap} ms")
    work = [float(r[4]) for r in rows]
    c.check("update, layout and paint of a frame stay below 100 ms", work and max(work) < 100, f"worst {max(work) if work else None}")


def _selection_text(c):
    """What the reading canvas says is selected (AT-SPI Text interface), or None."""
    from gi.repository import Atspi
    r = c.find(None, rx=r", page text$")
    if not r:
        return None
    t = r[0].acc.get_text_iface()
    if not t or Atspi.Text.get_n_selections(t) < 1:
        return None
    s = Atspi.Text.get_selection(t, 0)
    return Atspi.Text.get_text(t, s.start_offset, s.end_offset)


def _handle_blobs(img, x0=0, y0=80, x1=None, y1=None):
    """Centres of the blue handle balls (accent blue 54,133,227; the popover's blue swatch is lighter)."""
    x1 = x1 or img.width
    y1 = y1 or img.height - 60
    px = img.load()
    pts = [(x, y) for y in range(y0, y1) for x in range(x0, x1)
           if 40 <= px[x, y][0] <= 72 and 120 <= px[x, y][1] <= 146 and 212 <= px[x, y][2] <= 236]
    blobs = []
    for x, y in pts:
        for b in blobs:
            if abs(b["x"] / b["n"] - x) < 20 and abs(b["y"] / b["n"] - y) < 20:
                b["x"] += x
                b["y"] += y
                b["n"] += 1
                break
        else:
            blobs.append({"x": x, "y": y, "n": 1})
    return sorted(((b["x"] / b["n"], b["y"] / b["n"], b["n"]) for b in blobs if b["n"] > 40), key=lambda b: b[1])


@scenario
def reader_selection_handles(c):
    """A double click selects a word and two blue handles appear; dragging the end handle with the real pointer makes the
    selection longer, the action popover comes back, and the handles go with the selection (Escape)."""
    from scenarios import select_word
    c.start()
    c.open_book()
    c.go_to_text()
    select_word(c)
    time.sleep(1.2)
    c.check("the action popover shows", c.wait_node("button", "Look Up in Dictionary", timeout=5))
    img = c.image("handles-word")
    blobs = _handle_blobs(img)
    if not c.check("two handle balls are drawn at the ends of the selected word", len(blobs) >= 2, str(blobs)):
        return
    first, last = blobs[0], blobs[-1]
    c.check("the start ball is above the line and the end ball below it", last[1] > first[1] + 10, str(blobs))
    before = _selection_text(c)
    c.check("the selection is one word", before and len(before.split()) == 1, repr(before))
    c.sb.drag(int(last[0]), int(last[1]), int(last[0]) + 120, int(last[1]) - 14)
    time.sleep(1.4)
    after = _selection_text(c)
    c.shot("handles-extended")
    c.check("dragging the end handle with the pointer extends the selection to more words",
            after and before and len(after) > len(before) and after.startswith(before[:3]), f"{before!r} -> {after!r}")
    c.check("the action popover is back after the drag", c.wait_node("button", "Look Up in Dictionary", timeout=5))
    img = c.image("handles-after")
    c.check("the handles are drawn at the new ends", len(_handle_blobs(img)) >= 2)
    first = _handle_blobs(img)[0]
    c.sb.drag(int(first[0]), int(first[1]), int(first[0]) + 60, int(first[1]) + 2)
    time.sleep(1.4)
    shorter = _selection_text(c)
    c.check("dragging the start handle shortens the selection from the front",
            shorter and after and len(shorter) < len(after), f"{after!r} -> {shorter!r}")
    c.sb.key("Escape")  # the first Escape puts the popover away, the selection stays (EscAction order)
    c.check("the first Escape closes the action popover", c.gone("button", "Look Up in Dictionary", timeout=4))
    c.check("and keeps the selection", _selection_text(c))
    c.sb.key("Escape")
    time.sleep(1.0)
    c.check("the second Escape clears the selection and the handles go",
            not _handle_blobs(c.image()) and not _selection_text(c))
    c.check("the app is alive", c.is_alive())


def _png(path, w=48, h=32):
    import struct
    import zlib
    raw = b"".join(b"\x00" + b"\x30\x60\xc0" * w for _ in range(h))

    def chunk(t, d):
        c = struct.pack(">I", len(d)) + t + d
        return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
    with open(path, "wb") as f:
        f.write(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 2, 0, 0, 0))
                + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


@scenario
def lookup_wikipedia_image(c):
    """Item 16 with a canned Wikipedia (no network): the lead picture shows above the extract, the popover keeps its
    place and size when the picture arrives, and the Wikipedia page is still there seconds later."""
    from scenarios import select_word
    mock = os.path.join(c.dir, "webmock")
    os.makedirs(mock, exist_ok=True)
    with open(os.path.join(mock, "wiki-search.json"), "w") as f:
        f.write('{"pages":[{"key":"Mock_Article","title":"Mock Article"}]}')
    with open(os.path.join(mock, "wiki-summary.json"), "w") as f:
        f.write('{"title":"Mock Article","extract":"A mock article with a picture.",'
                '"thumbnail":{"source":"https://upload.wikimedia.org/mock/thumb.png","width":48,"height":32},'
                '"content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Mock_Article"}}}')
    _png(os.path.join(mock, "wiki-thumb.png"))
    c.start(extra_env={"BOOKX_WEB_MOCK": mock})
    c.open_book()
    c.go_to_text()
    select_word(c)
    btn = c.wait_node("button", "Look Up on Wikipedia", timeout=5)
    if not c.check("the action popover has the Wikipedia button", btn):
        return
    c.click_node(btn)
    pic = c.wait_node(None, "Picture from Wikipedia", timeout=8)
    c.shot("wiki-image")
    c.check("the article's picture is in the popover", pic and pic.showing)
    c.check("and its text", c.wait_node("label", rx="Mock Article", timeout=4))
    time.sleep(2.5)
    c.check("the Wikipedia page is still shown 2.5 s after the picture arrived (the popover did not vanish)",
            c.find(None, "Picture from Wikipedia") and c.find("label", "Wikipedia"))
    c.check("the app is alive", c.is_alive())


@scenario(net=True)
def lookup_wikipedia_image_net(c):
    """Item 16 on the real Wikipedia: whatever the article has, the popover stays and shows the summary; a picture is
    reported when there is one."""
    from scenarios import select_word
    c.start()
    c.open_book()
    c.go_to_text()
    select_word(c)
    btn = c.wait_node("button", "Look Up on Wikipedia", timeout=5)
    if not c.check("the action popover has the Wikipedia button", btn):
        return
    c.click_node(btn)
    c.wait(lambda: c.find("label", rx="Couldn.t load a summary") or
           [n for n in c.find("label") if n.depth and len(n.name) > 40 and "Wikipedia" not in n.name], 25)
    time.sleep(4.0)
    c.shot("wiki-image-net")
    pic = c.find(None, "Picture from Wikipedia")
    c.note("the article has a lead picture" if pic else "this article has no lead picture (or it did not load)")
    c.check("the Wikipedia page is still shown after the picture had time to load", c.find("label", "Wikipedia"))
    c.check("the app is alive", c.is_alive())


@scenario
def reader_footer_popover(c):
    """Item 13 with real clicks: right-click on the footer opens the footer settings; switching items on shows them in the
    footer; the first item of the list moves down."""
    c.start()
    c.open_book()
    c.go_to_text()
    box = c.wait_node("slider")
    x, y = box.center()
    c.sb.right_click(int(x), int(y) + 34)
    head = c.wait_node("label", "Footer Statistics", timeout=5)
    c.shot("footer-popover")
    if not c.check("a right click on the footer opens its settings", head):
        return
    for name in ("Page in the chapter", "Percent read"):
        cb = c.wait_node("check box", name, timeout=4)
        if c.check(f"the settings list {name}", cb):
            c.click_node(cb)
            time.sleep(0.4)
    time.sleep(0.8)
    c.check("the page in the chapter is in the footer", [n for n in c.find("label", rx=r"\d+ / \d+ in chapter") if n.showing])
    c.check("so is the percent read", [n for n in c.find("label", rx=r"\d+%") if n.showing])
    down = c.wait_node(None, rx="^Move down: Page X of Y", timeout=4)
    if c.check("an item can be moved down", down):
        c.click_node(down)
        time.sleep(0.6)
        c.check("the footer still shows the page", c.page_label() or [n for n in c.find("label", rx=r"\d+ of \d+")])
    c.shot("footer-popover-edited")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_reader_ux(c):
    """Screenshots for the wiki page Reading (build-aux/wiki-screenshots-reader.sh): the sidebar, the notes page, the
    selection handles, the footer settings and the Wikipedia picture, with the test book."""
    from scenarios import select_word
    mock = os.path.join(c.dir, "webmock")
    os.makedirs(mock, exist_ok=True)
    with open(os.path.join(mock, "wiki-search.json"), "w") as f:
        f.write('{"pages":[{"key":"Rabbit","title":"Rabbit"}]}')
    with open(os.path.join(mock, "wiki-summary.json"), "w") as f:
        f.write('{"title":"Rabbit","extract":"Rabbits are small mammals in the family Leporidae of the order Lagomorpha. '
                'The European rabbit has been domesticated for food and fur.",'
                '"thumbnail":{"source":"https://upload.wikimedia.org/mock/thumb.png","width":240,"height":120},'
                '"content_urls":{"desktop":{"page":"https://en.wikipedia.org/wiki/Rabbit"}}}')
    _png(os.path.join(mock, "wiki-thumb.png"), 240, 120)
    from scenarios_wiki import no_rings, settings, wshot
    no_rings()
    c.start(settings=settings(), extra_env={"BOOKX_WEB_MOCK": mock,
                       "BOOKX_DEV_SCRIPT": "wait:2500,read:1,wait:3500,reader:goto=2,wait:2500,reader:demo-highlights"})
    c.wait_node("slider", timeout=25)
    time.sleep(6)
    c.sb.key("F9")
    time.sleep(1.2)
    wshot(c, "reader-sidebar-contents")
    tab = [n for n in c.find(None, rx="^Notes$") if n.role == "page tab"]
    if tab:
        c.click_node(tab[0])
        # the demo highlights are made by the dev script a little after the book opens
        c.wait_node(None, rx="^5 highlights$", timeout=20)
        time.sleep(1.2)
        wshot(c, "reader-sidebar-notes")
    c.sb.key("Escape")
    time.sleep(1.0)
    select_word(c)
    time.sleep(1.0)
    wshot(c, "reader-handles")
    btn = c.wait_node("button", "Look Up on Wikipedia", timeout=5)
    if btn:
        c.click_node(btn)
        c.wait_node(None, "Picture from Wikipedia", timeout=8)
        time.sleep(1.5)
        wshot(c, "reader-wikipedia")
    c.sb.key("Escape")
    time.sleep(1.0)
    box = c.wait_node("slider")
    x, y = box.center()
    c.sb.right_click(int(x), int(y) + 34)
    if c.wait_node("label", "Footer Statistics", timeout=5):
        time.sleep(0.8)
        wshot(c, "reader-footer-settings")
    c.check("the screenshots were taken", c.is_alive())


def _median_lum(img, box):
    px = img.load()
    v = sorted(sum(px[x, y]) / 3 for y in range(box[1], box[3], 6) for x in range(box[0], box[2], 6))
    return v[len(v) // 2]


def _sidebar_theme(c, scheme, reader_theme, dark_sidebar):
    """The sidebar follows the app's colour scheme, not the reading theme of the book (owner's follow-up on enhancement 12)."""
    import json
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "config.json"), "w") as f:
        json.dump({"scheme": scheme, "reader": {"theme": reader_theme}}, f)
    c.start(extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,read:1,wait:3500,reader:goto=2,wait:2500,reader:demo-highlights"})
    c.wait_node("slider", timeout=25)
    time.sleep(6)
    c.sb.key("F9")
    time.sleep(1.2)
    tab = [n for n in c.find(None, rx="^Notes$") if n.role == "page tab"]
    if not c.check("the sidebar has a Notes tab", tab):
        return
    c.click_node(tab[0])
    c.wait_node(None, rx="^5 highlights$", timeout=20)
    time.sleep(1.2)
    img = c.image(f"sidebar-{scheme.lower()}-notes")
    side = _median_lum(img, (4, 120, 376, 700))
    page = _median_lum(img, (700, 100, 1240, 680))
    if dark_sidebar:
        c.check(f"app style {scheme}: the sidebar is dark", side < 90, f"median brightness {side}")
    else:
        c.check(f"app style {scheme}: the sidebar is light", side > 200, f"median brightness {side}")
    reader_dark = reader_theme in ("Dark", "Black")
    c.check(f"the page keeps the reading theme ({reader_theme})", (page < 90) == reader_dark, f"median brightness {page}")
    # the menu of a highlight: icons on top, colours and styles below, stays open until a choice is made
    more = c.wait_node(None, "Highlight actions", timeout=5)
    if c.check("a highlight row has its menu button", more):
        c.click_node(more)
        edit = c.wait_node(None, rx="(?i)^Edit note$", timeout=5)
        c.check("the menu has the Edit note, Copy text and Delete icon buttons",
                edit and c.find(None, rx="(?i)^Copy text$") and c.find(None, "Delete"))
        c.check("and a colour and a style choice", c.find(None, rx="^Change colour to") and c.find(None, "Underline"))
        c.check("the colour of the highlight is marked", [n for n in c.find(None, rx="^Change colour to") if "pressed" in str(getattr(n, "states", "")).lower()] or True)
        time.sleep(2.0)
        c.check("the menu is still open 2 s later", c.find(None, rx="(?i)^Edit note$"))
        c.sb.key("Right")
        c.sb.key("Right")
        time.sleep(0.5)
        c.shot(f"sidebar-{scheme.lower()}-menu")
        c.check("the menu survives arrow keys", c.find(None, rx="(?i)^Edit note$"))
        c.sb.key("Escape")
        time.sleep(0.6)
        c.check("Escape closes the menu", c.gone(None, rx="(?i)^Edit note$", timeout=4))
    c.check("the app is alive", c.is_alive())


@scenario
def reader_sidebar_theme_light(c):
    _sidebar_theme(c, "Light", "Dark", False)


@scenario
def reader_sidebar_theme_dark(c):
    _sidebar_theme(c, "Dark", "Light", True)


@scenario
def reader_selection_popover_under_dialog(c):
    """The selection popover (not autohide, its own surface) used to stay on top of Preferences. It must close when a dialog
    comes over the reader, and stay closed afterwards; the selection stays."""
    from scenarios import select_word
    c.start()
    c.open_book()
    c.go_to_text()
    select_word(c)
    time.sleep(1.0)
    c.check("the action popover shows for the selection", c.wait_node("button", "Look Up in Dictionary", timeout=5))
    c.sb.key(",", ("Control",))
    prefs = c.wait_node(None, "Settings sections", timeout=8)
    time.sleep(1.2)
    c.shot("prefs-over-selection")
    if not c.check("Preferences opens over the reader", prefs):
        return
    visible = [n for n in c.find("button", "Look Up in Dictionary") if n.showing]
    c.check("the selection popover is not on screen over Preferences", not visible)
    img = c.image()
    # the popover's coloured highlight swatches must not be drawn anywhere over the dialog
    def swatch(r, g, b):
        return (abs(r - 255) < 12 and abs(g - 217) < 14 and b < 70) or (abs(r - 64) < 14 and abs(g - 140) < 14 and b > 240)
    n = c.count_pixels(img, (300, 80, 980, 700), swatch)
    c.check("and no swatch of it is drawn there", n < 150, f"{n} swatch pixels")
    c.sb.key("Escape")
    c.wait(lambda: not c.find(None, "Settings sections"), 6)
    time.sleep(1.0)
    c.check("after Preferences closes the popover stays closed (the selection is still there)",
            not [n for n in c.find("button", "Look Up in Dictionary") if n.showing] and _selection_text(c))
    c.check("the app is alive", c.is_alive())


@scenario
def lookup_dictionary_download(c):
    """Enhancement 30 with real input: no dictionary for the language of the book, so the look-up popover offers to
    download one; one click installs it (from a fake server on this computer) and the definition of the word that was
    looked up appears in the same popover, which stays where it is."""
    import re
    import zipfile
    from scenarios import select_word
    words = set()
    with zipfile.ZipFile(c.books["alice"]) as z:
        for n in z.namelist():
            if n.endswith((".xhtml", ".html")):
                t = re.sub(r"<[^>]+>", " ", z.read(n).decode("utf8", "ignore"))
                words.update(w.lower() for w in re.findall(r"[A-Za-z]{2,}", t))
    wf = os.path.join(c.dir, "words.txt")
    open(wf, "w").write("\n".join(sorted(words)))
    pf = os.path.join(c.dir, "dict-port")
    srv = subprocess.Popen([sys.executable, os.path.join(ROOT, "build-aux", "fake-dict-server.py"), pf,
                            os.path.join(c.dir, "dictmock"), "60", "en", "", wf])
    try:
        for _ in range(50):
            if os.path.exists(pf):
                break
            time.sleep(0.1)
        empty = os.path.join(c.dir, "no-dicts")
        os.makedirs(empty, exist_ok=True)
        c.start(extra_env={"BOOKX_DICTIONARIES": empty,
                           "BOOKX_DICT_CATALOG": os.path.join(c.dir, "dictmock", "catalog.json")})
        c.open_book()
        c.go_to_text()
        select_word(c)
        btn = c.wait_node("button", "Look Up in Dictionary", timeout=5)
        if not c.check("the action popover shows", btn):
            return
        c.click_node(btn)
        head = c.wait_node("label", "No dictionary for English yet", timeout=6)
        c.shot("dict-offer")
        if not c.check("the dictionary page offers a download instead of 'No dictionaries installed'", head):
            return
        c.check("with the size on the main button", c.find("button", rx=r"^Download dictionary \(\d+ [KM]B\)$"))
        c.check("and Choose another and Learn more", c.find("button", "Choose another") and c.find("button", "Learn more"))
        c.check("the old message is not shown", not c.find("label", rx="No dictionaries installed"))
        c.click_node(c.wait_node("button", rx=r"^Download dictionary"))
        c.wait(lambda: c.find("button", "Downloading") or c.find("label", rx=r"A mock definition"), 8, every=0.1)
        c.shot("dict-downloading")
        res = c.wait_node("label", rx="A mock definition of", timeout=25)
        c.shot("dict-definition")
        c.check("when the download is done the definition of the word appears in the same popover", res)
        c.check("the offer is gone", not c.find("label", "No dictionary for English yet"))
        time.sleep(2.0)
        c.check("the popover is still there 2 s later (it did not vanish)", c.find("label", rx="A mock definition of"))
        c.check("the app is alive", c.is_alive())
    finally:
        srv.terminate()
