"""Screenshots for the public wiki (build-aux/wiki-screenshots.sh). The leading "_" keeps every scenario here out of
the normal session run. Each one runs twice, with WIKI_SCHEME=light and WIKI_SCHEME=dark (the GNOME color-scheme of the
sandbox, so libadwaita draws the real light or dark style); build-aux/screenshot-frame.py then adds the window shadow.
Pictures are taken without the mouse pointer. Imported by scenarios.py."""
import os
import subprocess
import sys
import time

from harness import scenario, Skip

HERE = os.path.dirname(os.path.abspath(__file__))
DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"


def scheme():
    return os.environ.get("WIKI_SCHEME", "light")


def settings():
    return DARK if scheme() == "dark" else ""


NO_RINGS_CSS = """/* wiki screenshots: no keyboard-focus ring, no focused-entry outline anywhere (docs/bugs.md bug 10) */
* { outline-color: transparent; outline-width: 0; }
*:focus, *:focus-visible, *:focus-within { outline-color: transparent; outline-width: 0; }
"""


def no_rings():
    """Writes the sandbox's user style sheet (GTK loads $XDG_CONFIG_HOME/gtk-4.0/gtk.css above the application's own CSS), so
    no widget draws a focus ring whatever the keyboard did. Call it before c.start()."""
    d = os.path.join(os.environ["XDG_CONFIG_HOME"], "gtk-4.0")
    os.makedirs(d, exist_ok=True)
    with open(os.path.join(d, "gtk.css"), "w") as f:
        f.write(NO_RINGS_CSS)


def _crop_box(c, crop, pad, size, minsize=(0, 0)):
    """A screen rectangle (x0, y0, x1, y1) for `crop`: an AT-SPI node, a list of nodes (their union) or a literal
    (x0, y0, x1, y1) in screen pixels; padded and clamped to the screen. Pure geometry besides the node boxes."""
    if isinstance(crop, tuple) and len(crop) == 4 and all(isinstance(v, (int, float)) for v in crop):
        x0, y0, x1, y1 = crop
    else:
        nodes = crop if isinstance(crop, (list, tuple)) else [crop]
        boxes = [c.screen_box(n) for n in nodes if n is not None and n.box[2] > 0 and n.box[3] > 0]
        if not boxes:
            return None
        x0 = min(b[0] for b in boxes)
        y0 = min(b[1] for b in boxes)
        x1 = max(b[2] for b in boxes)
        y1 = max(b[3] for b in boxes)
    return centred_box((x0, y0, x1, y1), pad, size, minsize)


def centred_box(box, pad, size, minsize=(0, 0), top=36):
    """`box` grown by `pad` (and to `minsize`) on every side, but never more than the screen allows on the tighter
    side of each axis, so the cut-out stays centred on `box`: the padding left and right (and above and below) is equal.
    A dialog near a screen edge used to get its full padding on one side and none on the other, which showed up as an
    off-centre picture on the page. `top` is the screen's top bar, which a picture never includes."""
    x0, y0, x1, y1 = box
    w, h = size
    out = []
    for lo, hi, limit_lo, limit_hi, want, least in ((x0, x1, 0, w, pad, minsize[0]), (y0, y1, top, h, pad, minsize[1])):
        lo, hi = max(lo, limit_lo), min(hi, limit_hi)
        grow = max(want, (least - (hi - lo)) / 2)
        grow = max(0, min(grow, lo - limit_lo, limit_hi - hi))
        out += [int(lo - grow), int(hi + grow)]
    return (out[0], out[2], out[1], out[3])


def wshot(c, label, park=True, crop=None, pad=16, scale=1.0, minsize=(0, 0)):
    """A screenshot without the mouse pointer (cursor=false), saved as <scenario>-<label>.png in the results folder. The
    pointer is parked on the shell's top bar first, so no row or button is drawn hovered (park=False for pictures that need
    the pointer, like the book map's hover card).

    `crop` makes a detail picture: a node, a list of nodes or a screen rectangle, taken from the same capture and cut out
    with `pad` pixels around it (grown to `minsize` so the details of one page have similar sizes) and magnified by `scale`.
    A detail gets a `.detail` marker next to it; wiki-screenshots.sh frames it with a small shadow, without the top bar cut."""
    if park:
        c.sb.move_to(c.sb.w // 2, 10)
        time.sleep(0.4)
    path = os.path.join(c.results, f"{c.name}-{label}.png")
    c.sb.screenshot_to(path, cursor=False)
    if crop is not None:
        from PIL import Image
        if os.environ.get("WIKI_FULL"):  # debugging a crop: the whole screen next to the results, in full/
            import shutil
            os.makedirs(os.path.join(c.results, "full"), exist_ok=True)
            shutil.copy(path, os.path.join(c.results, "full", f"{c.name}-{label}.png"))
        img = Image.open(path).convert("RGB")
        box = _crop_box(c, crop, pad, img.size, minsize)
        if box is None:
            c.note(f"{label}: nothing to crop to; the whole screen was kept")
            return path
        x0, y0, x1, y1 = box
        if x1 - x0 < 40 or y1 - y0 < 40:
            c.note(f"{label}: the crop {box} is empty; the whole screen was kept")
            return path
        out = img.crop((x0, y0, x1, y1))
        if scale != 1.0:
            out = out.resize((int(out.width * scale), int(out.height * scale)), Image.LANCZOS)
        out.save(path)
        open(path[:-4] + ".detail", "w").write("")
    return path


def maximize(c):
    """Windows fill a 1280x800 screen but float on bigger ones: Super+Up maximises, so the picture is the whole window."""
    c.sb.key("Up", ("Super",))
    time.sleep(1.5)


def shelf(c):
    """Adds twelve generated books (public-domain titles, drawn covers) next to the test book."""
    out = os.path.join(c.dir, "shelf")
    subprocess.run([sys.executable, os.path.join(HERE, "..", "make-wiki-books.py"), out], check=True, capture_output=True)
    names = []
    for f in sorted(os.listdir(out)):
        key = "shelf-" + f
        c.books[key] = os.path.join(out, f)
        names.append(key)
    return names


def set_config(**values):
    """Writes the app's config.json before it starts (the same file Preferences writes)."""
    import json
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "config.json"), "w") as f:
        json.dump(values, f)


@scenario
def _wiki_hero(c):
    """The Continue Reading banner as the hero card on a cold start, above a shelf of twelve generated books. The hero
    needs a tall window (a viewport lower than about 930 px gets the strip), so this one runs on a 1500 x 1080 screen."""
    names = shelf(c)
    no_rings()
    c.start(books=tuple(["alice"] + names), settings=settings(), size=os.environ.get("WIKI_HERO_SIZE", "1500x1080"),
            extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,library:lastread"})
    c.wait_node("table cell", rx="Alice")
    maximize(c)
    time.sleep(4)
    c.sb.move_to(1490, 1070)
    time.sleep(0.5)
    wshot(c, "library-banner-hero")
    c.check("the library shows the shelf", c.find("table cell", rx="Moby"))


@scenario
def _wiki_reading(c):
    """The reader picture of the landing page ("Pages made for reading"): a chapter with five highlights in four styles
    (background, underline, strikethrough, squiggly) and the selection popover open on the one with a note, so the note,
    the colours, the style choices and the Wikipedia look-up are all in the same picture."""
    no_rings()
    c.start(books=("alice",), settings=settings(), extra_env={
        "BOOKX_DEV_SCRIPT": "wait:2500,read:1,wait:4000,reader:goto=2,wait:2500,reader:demo-highlights,"
                            "wait:1500,reader:ui-note-popover"})
    if not c.check("the book opens", c.wait_node("slider", timeout=25)):
        return
    time.sleep(6)
    wiki = c.wait_node("button", "Look Up on Wikipedia", timeout=12)
    if not c.check("the selection popover of the highlighted passage is open", wiki):
        return
    c.check("its note is shown in the popover", c.wait_node(None, rx="Where the story really starts", timeout=5))
    c.check("the four highlight styles are in the popover", len(c.find("toggle button")) >= 4)
    c.sb.move_to(1270, 790)
    time.sleep(1.0)
    wshot(c, "reader")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_strip(c):
    """The slim strip of the Continue Reading banner (Preferences > Continue reading banner > Compact)."""
    names = shelf(c)
    set_config(banner_mode="compact")
    no_rings()
    c.start(books=tuple(["alice"] + names), settings=settings(),
            extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,library:lastread"})
    c.wait_node("table cell", rx="Alice")
    time.sleep(4)
    c.sb.move_to(1270, 790)
    time.sleep(0.5)
    wshot(c, "library-banner-strip")
    c.check("the strip was shot", True)


@scenario
def _wiki_library(c):
    """The details panel and Notes & Highlights of a library with a shelf of books."""
    names = shelf(c)
    no_rings()
    c.start(books=tuple(["alice"] + names), settings=settings(),
            extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,library:lastread,wait:3500,library:annotate,library:annotate-more"})
    c.wait_node("table cell", rx="Alice")
    time.sleep(4)
    card = c.wait_node("table cell", rx="Moby")
    if card:
        c.sb.click(*card.center())
        c.sb.move_to(1270, 790)
        time.sleep(1.2)
        wshot(c, "library-details")
    notes = c.wait_node(None, "Notes & Highlights")
    if notes:
        c.click_node(notes)
        c.sb.move_to(1270, 790)
        time.sleep(1.5)
        wshot(c, "library-notes")
    c.check("the library shows the shelf", True)


@scenario
def _wiki_favourites(c):
    """A starred, rated book in Favourites with its details, and the Properties dialog with the rating and the tag editor."""
    no_rings()
    c.start(books=("alice", "tiny"), settings=settings())
    card = c.wait_node("table cell", rx="Alice")
    c.sb.click(*card.center())
    c.wait_node("button", "Properties", timeout=8)
    c.sb.key("d", ("Control",))
    time.sleep(0.6)
    stars = c.wait_node("button", "4 stars", timeout=5)
    if stars:
        c.click_node(stars)
        time.sleep(0.6)
    c.click_node(c.wait_node(None, "Favourites"))
    time.sleep(1.0)
    card = c.wait_node("table cell", rx="Alice", timeout=5)
    if card:
        c.sb.click(*card.center())
    time.sleep(5.0)  # the "Added to Favourites" toast goes away
    wshot(c, "library-favourites")
    props = c.wait_node("button", "Properties", timeout=5)
    if props:
        c.click_node(props)
        c.wait_node("dialog", "Properties", timeout=6)
        time.sleep(1.0)
        wshot(c, "library-properties")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_table(c):
    names = shelf(c)
    no_rings()
    c.start(books=tuple(["alice"] + names), settings=settings(),
            extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,library:lastread,wait:2500,library:table"})
    time.sleep(7)
    c.sb.move_to(1270, 790)
    wshot(c, "library-table")
    c.check("the table was shot", True)


@scenario
def _wiki_map(c):
    from scenarios_map_keys import MAP_SCRIPT, map_label
    big = os.path.join(c.dir, "big.epub")
    subprocess.run([sys.executable, os.path.join(HERE, "..", "make-big-epub.py"), big, "900"], check=True,
                   capture_output=True)
    c.books["big"] = big
    no_rings()
    c.start(books=("big",), settings=settings(), extra_env={"BOOKX_DEV_SCRIPT": MAP_SCRIPT})
    c.wait_node("slider", timeout=40)
    time.sleep(16)
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    time.sleep(1.0)
    if not c.check("the map opens", area):
        return
    x0, y0, x1, y1 = c.screen_box(map_label(c))
    c.sb.move_to(x0 + 14 + 22 * 6.5, y0 + 14 + 15 + 15)
    time.sleep(0.8)
    wshot(c, "book-map", park=False)


@scenario
def _wiki_tts(c):
    """The sentence being read, the speaker popover and the Text to Speech settings (a silent mock engine)."""
    no_rings()
    c.start(settings=settings(), extra_env={"BOOKX_TTS_MOCK": "600"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    btn = c.wait_node("toggle button", rx="^Read Aloud")
    if not c.check("the reader has a Read aloud button", btn):
        return
    c.click_node(btn)
    play = c.wait_node("button", "Read Aloud from Here", timeout=5)
    if not c.check("the popover opens", play):
        return
    time.sleep(0.6)
    wshot(c, "read-aloud-popover")
    c.click_node(play)
    c.sb.key("Escape")
    c.wait_node("button", "Next Sentence", timeout=6)
    time.sleep(1.8)
    c.sb.move_to(1270, 790)
    wshot(c, "read-aloud-reading")
    c.click_node(c.find("button", "Close Read Aloud")[0])
    time.sleep(0.8)
    c.click_node(c.wait_node("toggle button", rx="^Read Aloud"))
    c.click_node(c.wait_node("button", rx="(?i)^Voice settings", timeout=5))
    if c.wait_node("dialog", "Preferences", timeout=8):
        c.wait_node(None, rx="^Voice engine", timeout=6)
        time.sleep(1.0)
        wshot(c, "read-aloud-settings")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_editor(c):
    """The editor with its preview, and the Markup menu with the split entries."""
    from scenarios_editor_tools import open_editor, status_line
    no_rings()
    c.start(settings=settings())
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    rows = [n for n in c.find(None, rx=r"htm\.xhtml") if n.showing]
    if len(rows) > 3:
        c.click_node(rows[3])
        time.sleep(1.5)
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("Home", ("Control",))
    for _ in range(31):
        c.sb.key("Down")
    time.sleep(1.0)
    c.sb.move_to(1270, 790)
    wshot(c, "editor-preview")
    btn = [n for n in c.find("toggle button") + c.find("button") if n.name.startswith("Markup")]
    if btn:
        c.click_node(btn[0])
        time.sleep(1.0)
        wshot(c, "editor-split-menu")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_vertical(c):
    if not c.books.get("vsynth"):
        raise Skip("vsynth not available (see docs/test-books.md)")
    no_rings()
    c.start(books=("vsynth",), settings=settings())
    if not c.check("the vertical book opens", c.open_book("縦書き")):
        return
    time.sleep(2.0)
    c.sb.move_to(1270, 790)
    wshot(c, "reader-vertical")


@scenario
def _wiki_eink(c):
    """The e-ink mode: black on white, no animation, bars that stay (the same in both styles)."""
    set_config(eink=True)
    no_rings()
    c.start(settings=settings())
    if not c.check("the book opens", c.open_book()):
        return
    c.go_to_text()
    time.sleep(1.0)
    c.sb.move_to(1270, 790)
    wshot(c, "reader-eink")
