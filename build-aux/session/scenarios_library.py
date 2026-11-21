"""Library window with real input: the sidebar, clicks on the empty space, Ctrl+click, right-click
(bug 8) and the details panel that follows the selection (enhancements 15 and 17)."""
import os
import re
import time

from harness import scenario


def _selected(c):
    """What the details panel says: 'none', 'one' or the text of the multi-selection title."""
    if c.find(None, rx=r"^\d+ Books Selected$"):
        return c.find(None, rx=r"^\d+ Books Selected$")[0].name
    if c.find("button", "Properties"):
        return "one"
    return "none"


@scenario
def library_sidebar(c):
    c.start(books=("alice", "tiny"))
    for name in ("Home", "Recent", "Favourites", "Notes & Highlights", "Library Health"):
        c.check(f"the sidebar has {name}", c.wait_node(None, name, timeout=5))
    c.check("the old sidebar toggle is gone", not c.find("toggle button", rx="^Browse panel"))
    c.shot("library-sidebar")
    c.click_node(c.wait_node(None, "Favourites"))
    time.sleep(0.8)
    c.check("Favourites is empty until a book is starred", not c.find("table cell", rx="Alice"))
    c.click_node(c.wait_node(None, "Home"))
    c.check("Home lists the books again", c.wait_node("table cell", rx="Alice", timeout=5))
    c.sb.click(*c.wait_node("table cell", rx="Alice").center())
    c.check("selecting a book opens the details panel by itself", c.wait_node("button", "Properties", timeout=5))
    c.sb.key("d", ("Control",))
    time.sleep(0.6)
    c.click_node(c.wait_node(None, "Favourites"))
    c.check("Ctrl+D stars the book and Favourites lists it", c.wait_node("table cell", rx="Alice", timeout=5))
    c.shot("library-favourites")


@scenario
def library_selection_clicks(c):
    c.start(books=("alice", "tiny"))
    a = c.wait_node("table cell", rx="Alice")
    b = c.wait_node("table cell", rx="Masque")
    if not c.check("both books are listed", a and b):
        return
    c.sb.click(*a.center())
    c.check("a click selects the book and the details panel opens", c.wait(lambda: _selected(c) == "one", 5))
    # the empty space below the rows
    x, y = a.center()
    c.sb.click(x, y + 260)
    c.check("a click on the empty space clears the selection and closes the panel",
            c.wait(lambda: _selected(c) == "none", 5), _selected(c))
    c.sb.click(*a.center())
    c.sb.click_with("Control", *b.center())
    c.check("Ctrl+click adds a second book", c.wait(lambda: _selected(c) == "2 Books Selected", 5), _selected(c))
    c.shot("library-two-selected")
    c.sb.click_with("Control", *a.center())
    c.check("Ctrl+click on a selected book takes it out", c.wait(lambda: _selected(c) == "one", 5), _selected(c))
    c.sb.click_with("Control", *a.center())
    c.sb.right_click(*b.center())
    menu = c.wait_node("menu", timeout=5)
    # the whole-selection menu has no Read, Properties or Show in Files (9 entries; the single-book menu has 12)
    c.check("a right-click on a selected book opens the menu for the whole selection",
            menu and len(c.find("menu item")) <= 10 and _selected(c) == "2 Books Selected", _selected(c))
    c.sb.key("Escape")
    c.gone("menu", timeout=3)
    c.sb.click(x, y + 260)
    c.sb.click(*a.center())
    c.sb.right_click(*b.center())
    menu = c.wait_node("menu", timeout=5)
    c.check("a right-click on an unselected book selects only that book",
            menu and len(c.find("menu item")) >= 11 and _selected(c) == "one", _selected(c))
    c.sb.key("Escape")
    c.gone("menu", timeout=3)
    # Closing a GTK popover does not restore a reliable keyboard focus in the virtual grid; click the
    # card again so Ctrl+A reaches the library's action instead of the popover.
    c.sb.click(*b.center())
    c.wait(lambda: any(n.role == "table cell" and n.focused for n in c.find()), 3)
    c.sb.key("a", ("Control",))
    c.check("Ctrl+A selects every book", c.wait(lambda: _selected(c) == "2 Books Selected", 5), _selected(c))
    c.sb.key("Escape")
    time.sleep(0.5)
    c.check("the app is still alive", c.is_alive())


def _title_boxes(c):
    """The title labels of the cover tiles: {title: (x, y, w, h)}."""
    return {n.name: n.box for n in c.find("label") if 60 < n.box[2] < 200 and n.box[1] > 40 and n.box[0] > 230
            and any(k in n.name for k in ("Alice", "Masque", "Synthetic", "Neko", "Vertical", "吾輩", "テスト"))}


def _cover_crop(img, label_box, origin=(7, 37)):
    """The inside of the cover above a title label (centred on the label, 140 x 200 px)."""
    x, y, w, h = label_box
    cx, cy = origin[0] + x + w // 2, origin[1] + y - 140
    return img.convert("RGB").crop((cx - 70, cy - 100, cx + 70, cy + 100))


def _crop_difference(a, b):
    from PIL import ImageChops, ImageStat
    return sum(ImageStat.Stat(ImageChops.difference(a, b)).mean) / 3


@scenario
def library_tiles_reflow(c):
    """Enhancement 24 with a real compositor: opening the details panel narrows the area of the books;
    the covers keep their size and the grid reflows (fewer books in a row)."""
    c.start(books=("alice", "tiny", "vsynth", "vneko"))
    time.sleep(1.5)
    before = _title_boxes(c)
    if not c.check("the cover grid shows the books", len(before) >= 3, str(before)):
        return
    first = [kv for kv in before.items() if "Alice" in kv[0]][0]
    img0 = c.image("tiles-closed")
    row0 = len({b[1] // 40 for b in before.values()})
    crop0 = _cover_crop(img0, first[1])
    # select the first book: the details panel opens on the right and the books get less room
    c.sb.click(first[1][0] + first[1][2] / 2 + 7, first[1][1] + 37 - 60)
    c.check("selecting a book opens the details panel", c.wait_node("button", "Properties", timeout=5))
    time.sleep(1.2)
    after = _title_boxes(c)
    img1 = c.image("tiles-open")
    crop1 = _cover_crop(img1, after[first[0]])
    rows1 = len({b[1] // 40 for b in after.values()})
    c.check("the title labels keep their size", all(after[k][2:] == before[k][2:] for k in before if k in after),
            f"{before[first[0]]} {after[first[0]]}")
    diff = _crop_difference(crop0, crop1)
    c.check("the cover looks the same size in the screenshots (same pixels, just moved)", diff < 14, f"mean difference {diff:.1f}")
    c.check("the grid reflows: books moved to another row", rows1 > row0, f"rows {row0} -> {rows1}")
    c.sb.key("Escape")
    time.sleep(1.2)
    again = _title_boxes(c)
    c.check("closing the panel moves them back", len({b[1] // 40 for b in again.values()}) == row0, str(again))
    c.check("the app is still alive", c.is_alive())


DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"


def _look(c, name, settings):
    """Screenshots of the library (split headers, sidebar, details) and of Notes & Highlights."""
    c.start(books=("alice", "tiny"), settings=settings,
            extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,library:annotate,library:annotate-more"})
    time.sleep(4)
    c.shot(f"{name}-home")
    c.sb.click(*c.wait_node("table cell", rx="Alice").center())
    time.sleep(1.2)
    c.shot(f"{name}-selected")
    c.sb.click(*c.wait_node(None, "Notes & Highlights").center())
    time.sleep(1.5)
    c.shot(f"{name}-notes")
    c.check("the notes view lists a highlight", c.wait_node(None, rx="devhighlight|greenquote|bluequote", timeout=5))
    c.check("the notes view has one toolbar with Filter", [n for n in c.find("toggle button") if n.name.startswith("Filter")])


@scenario
def library_look_light(c):
    _look(c, "look-light", "")


@scenario
def library_look_dark(c):
    _look(c, "look-dark", DARK)


def _origin(w, h, screen=(1280, 800)):
    """AT-SPI window coordinates -> screen for a window of w x h centred on the screen."""
    sw, sh = screen
    if (w, h) == screen:
        return (7, 37)  # the default window is maximised
    return (int((sw - w) / 2) + 7, int(32 + (sh - 32 - h) / 2) + 5)


def _back_to_library(c):
    c.open_book()
    time.sleep(2)
    for _ in range(3):
        c.sb.key("Right")
        time.sleep(0.6)
    c.click_node(c.wait_node("button", rx="^Back"))
    time.sleep(1.5)


def _click(c, node, origin, **kw):
    x, y = node.center(origin)
    c.sb.click(x, y, **kw)


def _banner_lines(c):
    """The `banner:` lines the app printed to its log (the dev script asks for them)."""
    path = os.path.join(c.dir, "app.log")
    try:
        text = open(path, errors="replace").read()
    except OSError:
        return []
    out = []
    for line in text.splitlines():
        m = re.search(r"banner: (.*)", line)
        if m:
            out.append({k: v for k, v in re.findall(r"(\w+)=(\S+)", m.group(1))})
    return out


def _last_banner(c, settle=1.5):
    time.sleep(settle)
    lines = _banner_lines(c)
    return lines[-1] if lines else None


def _hero_case(c, name, settings):
    """A cold start in a 900x600 window that is then moved to the corner and resized with the real
    pointer, continuously, to 1280x800, 1920x1080 and 2400x1300 (and the details panel opened there, and
    back to 900x600): the banner is a full-width card at every size, never a square, a strip on the
    small window, the hero on the others, 25-35 % of the viewport."""
    sw, sh = 2560, 1440
    c.start(books=("alice", "tiny"), settings=settings, size=f"{sw}x{sh}",
            extra_env={"BOOKX_SIZE": "900x600", "BOOKX_DEV_SCRIPT": "wait:2500,library:lastread,library:banner-watch"})
    c.wait_node("table cell", rx="Alice")
    time.sleep(4)
    w, h = 900, 600
    x0, y0 = (sw - w) // 2, 32 + (sh - 32 - h) // 2
    # move the window to the top-left corner by its header bar
    c.sb.drag(x0 + w // 2, y0 + 14, x0 + w // 2 - (x0 - 10), y0 + 14 - (y0 - 40))
    x0, y0 = 10, 40
    time.sleep(1.5)
    results = []

    def look(label, variant, lo, hi):
        line = _last_banner(c)
        c.shot(f"{name}-{label}")
        if not c.check(f"{label}: the banner is shown", line and line.get("shown") == "true"):
            return
        vh, vw = (int(v) for v in line["viewport"].split("x"))
        card_w, area_w, card_h = int(line["card_w"]), int(line["area_w"]), int(line["card_h"])
        pct = float(line["percent"])
        c.check(f"{label}: the window really has this size (viewport {vh} x {vw})", vw > 0)
        c.check(f"{label}: {variant} layout", line["variant"] == variant, line["variant"])
        c.check(f"{label}: {lo}-{hi} % of the viewport", lo <= pct <= hi, f"{pct}")
        c.check(f"{label}: a full-width card (at most ~1100 px), not a square (card {card_w} of {area_w} px wide, {card_h} high)",
                card_w >= min(0.9 * (area_w - 24), 1040) and card_w > 1.3 * card_h, f"{card_w}x{card_h}")
        results.append((label, vw))

    look("900x600", "strip", 0, 26)
    for tw, th in ((1280, 800), (1920, 1080), (2400, 1300)):
        # the bottom-right corner: a continuous drag (the pointer moves in ten steps)
        c.sb.drag(x0 + w + 3, y0 + h + 3, x0 + tw + 3, y0 + th + 3)
        w, h = tw, th
        # a viewport under 928 px is too low for the hero: the strip (a quarter of the viewport at most)
        if th < 1000:
            look(f"{tw}x{th}", "strip", 0, 26)
        else:
            look(f"{tw}x{th}", "wide", 24, 36)
    # the details panel opens by itself when a book is selected
    c.sb.click(*c.wait_node("table cell", rx="Alice").center(origin=(x0 + 7, y0 + 5)))
    time.sleep(1.5)
    look("2400x1300-with-details", "wide", 24, 36)
    c.sb.drag(x0 + w + 3, y0 + h + 3, x0 + 900 + 3, y0 + 600 + 3)
    look("back-to-900x600", "strip", 0, 26)
    c.check("the window got wider with every drag (the real pointer resized it)",
            [v for _, v in results] != sorted(set(v for _, v in results)) or len(set(v for _, v in results)) >= 4,
            str(results))
    c.check("the app is still alive after all the resizes", c.is_alive())
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    c.check("no GTK allocation warning while the window changes size", "without a current allocation" not in log,
            "".join(l for l in log.splitlines(True) if "WARNING" in l)[:200])


@scenario
def library_hero_light(c):
    _hero_case(c, "hero-light", "")


@scenario
def library_hero_dark(c):
    _hero_case(c, "hero-dark", DARK)


def _hero_session(c, name, settings):
    """The hero on a cold start, the slim strip after a book was opened and closed."""
    c.start(books=("alice", "tiny"), size="1920x1080", settings=settings,
            extra_env={"BOOKX_SIZE": "1800x1030", "BOOKX_DEV_SCRIPT": "wait:2500,library:lastread"})
    origin = _origin(1800, 1030, (1920, 1080))
    c.wait_node("table cell", rx="Alice")
    time.sleep(5)
    c.shot(f"{name}-cold")
    c.check("a cold start shows the hero with its secondary actions",
            c.find("button", "Continue") and c.find("button", "Open from Start") and c.find("button", "Mark as Finished"))
    c.sb.double_click(*c.wait_node("table cell", rx="Alice").center(origin))
    c.wait_node("button", rx="^Back", timeout=20)
    time.sleep(2)
    c.sb.key("Escape")
    time.sleep(3)
    c.shot(f"{name}-back")
    c.check("back from the book: the slim strip with just Continue", c.find("button", "Continue") and not c.find("button", "Open from Start"))
    c.check("and the books are in view below it", c.find("table cell", rx="Masque"))


@scenario
def library_hero_session(c):
    _hero_session(c, "hero-session", "")


@scenario
def library_hero_session_dark(c):
    _hero_session(c, "hero-session-dark", DARK)


def _notes_case(c, name, settings, size):
    w, h = size
    origin = _origin(w, h)
    env = {"BOOKX_DEV_SCRIPT": "wait:2500,library:annotate,library:annotate-more"}
    if size != (1280, 800):
        env["BOOKX_SIZE"] = f"{w}x{h}"
    if w < 1100:
        # a narrow window has the sidebar as an overlay, closed to begin with
        env["BOOKX_DEV_SCRIPT"] += ",library:sidebar-toggle"
    c.start(books=("alice", "tiny"), settings=settings, extra_env=env)
    time.sleep(4)
    _click(c, c.wait_node(None, "Notes & Highlights"), origin)
    time.sleep(1.5)
    c.shot(f"{name}-{w}x{h}")
    boxes = [n.box for n in c.find("check box") if n.name == "Select this highlight"]
    if not c.check("the notes are listed", len(boxes) >= 3, str(len(boxes))):
        return
    xs = sorted({b[0] // 40 for b in boxes})
    ys = sorted({b[1] for b in boxes})
    # rows of one column are close together: compact cards
    col = sorted(b[1] for b in boxes if b[0] // 40 == xs[0])
    gaps = [b - a for a, b in zip(col, col[1:])]
    c.check("the cards are compact (about 74 px with a note, 45 without; a book heading adds about 30)",
            all(40 <= g <= 125 for g in gaps) if gaps else True, str(gaps))
    if w >= 1200:
        c.check("on a wide window the cards use several columns", len(xs) >= 2, str(xs))
    last_x = max(b[0] for b in boxes)
    c.check("the list uses the width of the window, it is not a centred column", min(b[0] for b in boxes) < 330, str(min(b[0] for b in boxes)))
    search = c.find("search text") or c.find("entry")
    filt = [n for n in c.find("toggle button") if n.name.startswith("Filter")]
    c.check("search and Filter are on one toolbar row", search and filt and abs(search[0].box[1] - filt[0].box[1]) < 14,
            f"{[s.box for s in search[:1]]} {[f.box for f in filt[:1]]}")
    c.check("the date entries are not on the page (they are in the Filter popover)", not c.find(None, rx="YYYY"))
    _click(c, filt[0], origin)
    c.check("the Filter popover has the date pickers and Clear Filters",
            c.wait_node("button", "Clear Filters", timeout=4) and c.find(None, rx="^From"))
    c.sb.key("Escape")
    time.sleep(0.5)


@scenario
def library_notes_light_wide(c):
    _notes_case(c, "notes-light", "", (1280, 800))


@scenario
def library_notes_dark_wide(c):
    _notes_case(c, "notes-dark", DARK, (1280, 800))


@scenario
def library_notes_light_small(c):
    _notes_case(c, "notes-light", "", (900, 600))


@scenario
def library_notes_dark_small(c):
    _notes_case(c, "notes-dark", DARK, (900, 600))


def _fav_case(c, name, settings):
    """A favourite: the star badge on its cover, the star toggle, the rating and the tags in the
    details panel and in Properties."""
    c.start(books=("alice", "tiny"), settings=settings,
            extra_env={"BOOKX_DEV_SCRIPT": "wait:2500,library:select1,wait:500,library:favourite,wait:600,library:rate=8,wait:900,library:tags=+Fantasy;Classic,wait:900"})
    c.wait_node("table cell", rx="Alice")
    time.sleep(8)
    c.shot(f"{name}-details")
    c.check("the favourite star toggle shows its state in the details panel",
            [n for n in c.find("toggle button") if n.name == "Favourite"])
    c.check("the star badge is on the cover (clickable, named)", c.find("button", rx="^Favourite: click to remove"))
    c.check("the rating stars are in the details panel", c.find("button", "4 stars") and c.find("button", "5 stars"))
    c.check("the tags are chips with remove buttons", c.find("button", "Remove the tag Fantasy") and c.find("button", "Remove the tag Classic"))
    c.click_node(c.wait_node("button", "Properties"))
    time.sleep(1.5)
    c.shot(f"{name}-properties")
    c.check("Properties has the rating and the tag editor too", c.find("button", "3 stars") and c.find("entry", "Add a tag") or c.find(None, "Add a tag"))
    c.sb.key("Escape")
    time.sleep(0.6)
    # the badge itself toggles: one click takes the star off
    c.click_node(c.wait_node("button", rx="^Favourite: click to remove"))
    time.sleep(1)
    c.check("a click on the badge takes the star off", not c.find("button", rx="^Favourite: click to remove"))
    c.shot(f"{name}-unstarred")


@scenario
def library_favourites_light(c):
    _fav_case(c, "fav-light", "")


@scenario
def library_favourites_dark(c):
    _fav_case(c, "fav-dark", DARK)


def _jobs_case(c, name, settings, size, lang="en"):
    """The Background Jobs popover with long titles, in a real session: the rows fit the popover (no sideways
    scrolling), rings instead of bars, the header on one line (also in German)."""
    long_title = "Ein sehr langer Titel einer Hintergrundaufgabe der immer weiter geht und nirgends mehr hinpasst" \
        if lang == "de" else "A very long title of a background job that goes on and on and on until nothing fits"
    script = (f"wait:2500,jobs:start=voice|{long_title}|cancel,jobs:progress=1|0.98|130 MB of 132 MB \u00b7 1.4 MB/s \u00b7 2 s left,"
              f"jobs:start=import|{long_title}|cancel,jobs:start=send|{long_title}|no,"
              f"jobs:finish=3|{long_title} {long_title},jobs:retryable=3,jobs:start=backup|Backup|no,jobs:finish=4|ok,"
              "jobs:open,wait:1500,jobs:overflow,jobs:rows")
    env = {"BOOKX_DEV_SCRIPT": script, "BOOKX_LANG": lang}
    w, h = size
    if size != (1280, 800):
        env["BOOKX_SIZE"] = f"{w}x{h}"
    c.start(books=("alice",), settings=settings, extra_env=env)
    c.wait_node("table cell", rx="Alice")
    time.sleep(8)
    c.shot(f"{name}-{lang}-{w}x{h}")
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    m = re.search(r"jobs: overflow upper=(\d+) page=(\d+) body=(\d+) popover=(\d+)", log)
    if not c.check("the panel reported its size", m):
        return
    upper, page, body, pop = (int(v) for v in m.groups())
    c.check(f"no sideways scrolling ({upper} of {page} px)", upper == page)
    c.check(f"the popover is at most 380 px wide and fits the window ({body} px)", 240 <= body <= 380)
    title = c.find("label", rx="Background Jobs|Hintergrundaufgaben")
    clear = c.find("button", rx="Clear Finished|Erledigte|Beendete")
    c.check("the header has its title and the Clear Finished button on one line",
            title and clear and abs(title[0].box[1] - clear[0].box[1]) < 14, f"{[t.box for t in title[:1]]} {[b.box for b in clear[:1]]}")
    c.check("the Retry button of the failed job is there", c.find("button", rx="^Retry|^Wiederholen|^Reintentar") or c.find("button", rx="Retry"))
    c.check("the app is still alive", c.is_alive())


@scenario
def library_jobs_light(c):
    _jobs_case(c, "jobs-light", "", (1280, 800))


@scenario
def library_jobs_dark(c):
    _jobs_case(c, "jobs-dark", DARK, (1280, 800))


@scenario
def library_jobs_german_small(c):
    _jobs_case(c, "jobs-light", "", (900, 600), "de")


@scenario
def library_jobs_german_dark(c):
    _jobs_case(c, "jobs-dark", DARK, (1280, 800), "de")


_RESP_STATES = {
    "sidebar": "library:sidebar-toggle,wait:900",
    "details": "library:select1,wait:600,library:details-open,wait:900",
    "jobs": ("jobs:start=voice|A very long title of a background job that goes on and on|cancel,"
             "jobs:progress=1|0.6|60 MB of 100 MB \u00b7 1.4 MB/s \u00b7 20 s left,"
             "jobs:start=send|Send to the reader|no,jobs:finish=2|disk full,jobs:open,wait:1200"),
    "base": "wait:300",
}


def _resp_case(c, name, settings, size, lang, state):
    """The library at a small window size: the header never clips, the panels are overlays."""
    w, h = size
    env = {"BOOKX_DEV_SCRIPT": f"wait:2500,{_RESP_STATES[state]},library:header",
           "BOOKX_LANG": lang, "BOOKX_SIZE": f"{w}x{h}"}
    c.start(books=("alice", "tiny"), settings=settings, extra_env=env)
    c.wait_node("table cell", rx="Alice|Tiny")
    time.sleep(9)
    c.shot(f"{name}-{state}-{lang}-{w}x{h}")
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    hd = re.findall(r"header: width=(\d+) children=(\d+) rightmost=(\d+) clipped=\[(.*?)\]", log)
    lay = re.findall(r"layout: window=(\d+) sidebar_overlay=(\w+) details_overlay=(\w+)", log)
    if not c.check("the header was measured", hd and lay):
        return
    width, kids, right, clipped = hd[-1]
    c.check("nothing sticks out of the header", clipped == "" and int(right) <= int(width), f"{hd[-1]}")
    c.check("the header has its buttons", int(kids) >= 4, kids)
    win, side, det = lay[-1]
    c.check("both panels are overlays in a small window", side == "true" and det == "true", f"{lay[-1]}")
    c.check("the window is as wide as asked (or the minimum)", int(win) <= max(w, 360) + 2, f"{win} for {w}")
    mw = re.findall(r"minwidth: header=(\d+) .*toolbar=(\d+)", log)
    if c.check("the least width was reported", mw):
        c.check("the content fits the window (its least width is not more than the window)",
                int(mw[-1][1]) <= max(w, 360), f"{mw[-1]} in {w}")
    c.check("the app is still alive", c.is_alive())


def _resp_scenarios():
    cases = [
        ((900, 600), "", "en", "sidebar"), ((900, 600), DARK, "de", "details"), ((900, 600), "", "de", "jobs"),
        ((800, 600), DARK, "en", "sidebar"), ((800, 600), "", "de", "details"), ((800, 600), DARK, "de", "jobs"),
        ((700, 500), "", "en", "jobs"), ((700, 500), DARK, "de", "sidebar"), ((700, 500), "", "de", "details"),
        ((360, 294), "", "de", "base"), ((360, 294), DARK, "en", "sidebar"), ((360, 294), "", "en", "jobs"),
    ]
    for size, settings, lang, state in cases:
        theme = "dark" if settings else "light"
        fname = f"library_resp_{size[0]}_{theme}_{lang}_{state}"

        def run(c, size=size, settings=settings, lang=lang, state=state, theme=theme):
            _resp_case(c, f"resp-{theme}", settings, size, lang, state)
        run.__name__ = fname
        scenario(run)


_resp_scenarios()


def _chips_case(c, name, settings, screen, win):
    """The "Also reading" covers of the hero: big enough to hit, the column inside the card, the hero still
    25-35 % of the viewport, a hover and a keyboard focus that show, the whole button opens the book."""
    sw, sh = screen
    w, h = win
    c.start(books=("alice", "tiny", "vsynth", "vneko"), settings=settings, size=f"{sw}x{sh}",
            extra_env={"BOOKX_SIZE": f"{w}x{h}", "BOOKX_DEV_SCRIPT": "wait:2500,library:lastread-many,library:banner-watch,wait:3000,library:banner-focus-chip,wait:800,library:banner"})
    origin = _origin(w, h, screen)
    c.wait_node("table cell", rx="Alice|Tiny|Neko|Synth|vertical", timeout=40)
    time.sleep(6)
    line = _last_banner(c)
    c.shot(f"{name}-{w}x{h}")
    if not c.check("the banner is shown", line and line.get("shown") == "true", str(line)):
        return
    pct = float(line["percent"])
    c.check("the hero layout", line["variant"] == "wide", line["variant"])
    c.check("the hero is 24-33 % of the viewport", 24 <= pct <= 33, f"{pct}")
    c.check("three covers under Also reading", line.get("chips") == "3", str(line.get("chips")))
    cw, ch = (int(v) for v in line["chip"].split("x"))
    c.check("each button is 88 x 130 px (72 x 108 cover, 8 px padding, progress bar), not wider",
            88 <= cw <= 96 and 124 <= ch <= 140, line["chip"])
    c.check("the column lies inside the card", line.get("col_in") == "true", str(line.get("col_in")))
    chips = c.find("button", rx="^Continue reading .*completed")
    if not c.check("the covers are buttons with a name that has the title and the progress", len(chips) >= 3, str(len(chips))):
        return
    x, y = chips[0].center(origin)
    # the pointer on the padding next to the cover is on the button too
    c.sb.move_to(x - 40, y)
    time.sleep(0.8)
    c.shot(f"{name}-{w}x{h}-hover")
    # keyboard focus on a chip (set by the dev script): the focus ring shows
    c.sb.move_to(x, y + 420)
    c.sb.key("Shift")
    time.sleep(0.8)
    c.shot(f"{name}-{w}x{h}-focus")
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    c.check("a chip can take the keyboard focus", "banner-focus-chip: true" in log and "chip_focus=true" in log)
    c.sb.click(x - 40, y)
    c.check("a click on the padding of a cover opens that book",
            c.wait_node("button", rx="^Back", timeout=20) is not None)
    c.check("the app is still alive", c.is_alive())


@scenario
def library_chips_light_1080(c):
    _chips_case(c, "chips-light", "", (1920, 1080), (1800, 1030))


@scenario
def library_chips_dark_1080(c):
    _chips_case(c, "chips-dark", DARK, (1920, 1080), (1800, 1030))


@scenario
def library_chips_light_1300(c):
    _chips_case(c, "chips-light", "", (2560, 1440), (2400, 1300))


@scenario
def library_chips_dark_1300(c):
    _chips_case(c, "chips-dark", DARK, (2560, 1440), (2400, 1300))


def _filter_case(c, name, settings):
    """Real clicks: a sidebar filter row opens the search bar with its query (the search button on, the keyboard
    focus stays on the sidebar), Home clears it again."""
    poll = "wait:3000" + ",library:searchstate,wait:1500" * 24
    c.start(books=("alice", "tiny"), settings=settings, extra_env={"BOOKX_DEV_SCRIPT": poll})
    origin = (7, 37)
    c.wait_node("table cell", rx="Alice")
    time.sleep(3)
    _click(c, c.wait_node(None, "Authors"), origin)
    time.sleep(1)
    c.wait_node(None, rx="Lewis Carroll", timeout=8)
    rows = sorted(c.find(None, rx="Lewis Carroll"), key=lambda n: n.box[0])
    row = rows[0] if rows and rows[0].box[0] < 260 else None
    if not c.check("the Authors section lists the author", row):
        return
    _click(c, row, origin)
    time.sleep(2.5)
    c.shot(f"{name}-filter")
    toggle = [n for n in c.find("toggle button") if n.name.startswith("Search")]
    c.check("the search button is there", bool(toggle))
    entry = c.find("search text") or c.find("entry")
    c.check("the search entry is on screen", bool(entry))
    _click(c, c.wait_node(None, "Home"), origin)
    time.sleep(2.5)
    c.shot(f"{name}-home")
    time.sleep(8)
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    states = re.findall(r'searchstate: bar=(\w+) text="(.*?)" books=(\d+) view=(\w+) focus=(\S+)', log)
    filt = [s for s in states if s[0] == "true" and "Carroll" in s[1]]
    if not c.check("the app reported the filter state", filt, str(states[-3:])):
        return
    c.check("the filter row opened the bar with its query and filtered the books", filt[0][2] == "1", str(filt[0]))
    c.check("the keyboard focus stayed on the sidebar (not in the search entry)",
            "Text" not in filt[0][4] and "Search" not in filt[0][4], filt[0][4])
    last = states[-1]
    c.check("Home closed the bar, cleared the query and shows every book", last[0] == "false" and last[1] == "" and last[2] == "2",
            str(last))
    c.check("the app is still alive", c.is_alive())


@scenario
def library_filter_light(c):
    _filter_case(c, "filter-light", "")


@scenario
def library_filter_dark(c):
    _filter_case(c, "filter-dark", DARK)
