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


def wshot(c, label):
    """A screenshot without the mouse pointer, saved as <scenario>-<label>.png in the results folder."""
    path = os.path.join(c.results, f"{c.name}-{label}.png")
    c.sb.screenshot_to(path, cursor=False)
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
    c.start(books=("alice",), settings=settings(), extra_env={
        "BOOKX_DEV_SCRIPT": "wait:2500,read:1,wait:4000,reader:goto=2,wait:2500,reader:demo-highlights,"
                            "wait:1500,reader:ui-note-popover"})
    if not c.check("the book opens", c.wait_node("slider", timeout=25)):
        return
    time.sleep(6)
    wiki = c.wait_node("button", "Look up on Wikipedia", timeout=12)
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
def _wiki_table(c):
    names = shelf(c)
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
    wshot(c, "book-map")


@scenario
def _wiki_tts(c):
    """The sentence being read, the speaker popover and the Text to Speech settings (a silent mock engine)."""
    c.start(settings=settings(), extra_env={"BOOKX_TTS_MOCK": "600"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    btn = c.wait_node("toggle button", rx="^Read aloud")
    if not c.check("the reader has a Read aloud button", btn):
        return
    c.click_node(btn)
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens", play):
        return
    time.sleep(0.6)
    wshot(c, "read-aloud-popover")
    c.click_node(play)
    c.sb.key("Escape")
    c.wait_node("button", "Next sentence", timeout=6)
    time.sleep(1.8)
    c.sb.move_to(1270, 790)
    wshot(c, "read-aloud-reading")
    c.click_node(c.find("button", "Close read aloud")[0])
    time.sleep(0.8)
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    c.click_node(c.wait_node("button", rx="^Voice settings", timeout=5))
    if c.wait_node("dialog", "Preferences", timeout=8):
        c.wait_node(None, rx="^Voice engine", timeout=6)
        time.sleep(1.0)
        wshot(c, "read-aloud-settings")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_editor(c):
    """The editor with its preview, and the Markup menu with the split entries."""
    from scenarios_editor_tools import open_editor, status_line
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
    btn = [n for n in c.find("toggle button") + c.find("button") if n.name.startswith("Markup:")]
    if btn:
        c.click_node(btn[0])
        time.sleep(1.0)
        wshot(c, "editor-split-menu")
    c.check("the app is alive", c.is_alive())


@scenario
def _wiki_vertical(c):
    if not c.books.get("vsynth"):
        raise Skip("vsynth not available (see docs/test-books.md)")
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
    c.start(settings=settings())
    if not c.check("the book opens", c.open_book()):
        return
    c.go_to_text()
    time.sleep(1.0)
    c.sb.move_to(1270, 790)
    wshot(c, "reader-eink")
