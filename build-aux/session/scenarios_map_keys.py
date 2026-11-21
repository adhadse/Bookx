"""Book map and the well-known reader keys with real input (enhancements 18 and 21). Imported by scenarios.py."""
import os
import re
import subprocess
import sys
import time

from harness import scenario

HERE = os.path.dirname(os.path.abspath(__file__))


def map_label(c):
    """The accessible name of the map: 'Book map: <level>, row N of M, <chapter> · <page>'."""
    for n in c.find(None, rx=r"^Book map: (Whole book|Sections|Single pages)"):
        return n
    return None


def row_of(n):
    m = re.search(r"row (\d+) of (\d+)", n.name) if n else None
    return (int(m.group(1)), int(m.group(2))) if m else None


@scenario
def new_reader_keys(c):
    """M opens the book map, Esc closes it, Ctrl+W goes back to the library (keys from other readers)."""
    c.start()
    c.open_book()
    c.go_to_text()
    c.sb.key("m")
    ok = c.wait(lambda: map_label(c), 6)
    c.shot("after-m")
    c.check("M opens the book map", ok, str([n.name for n in c.find(None, rx="(?i)map")][:5]))
    c.sb.key("Escape")
    c.check("Esc closes it", c.wait(lambda: not map_label(c), 5))
    c.sb.key("w", ("Control",))
    c.check("Ctrl+W goes back to the library", c.wait_node("table cell", rx="Alice", timeout=8))


@scenario
def book_map_wheel_drag(c):
    """The wheel scrolls the map, Ctrl+wheel zooms it, a drag through the book goes to the page under the pointer."""
    big = os.path.join(c.dir, "big.epub")
    subprocess.run([sys.executable, os.path.join(HERE, "..", "make-big-epub.py"), big, "900"], check=True,
                   capture_output=True)
    c.books["big"] = big
    c.start(books=("big",))
    c.open_book(title_rx="Big Book")
    time.sleep(2.0)
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    if not c.check("M opens the book map", area):
        return
    time.sleep(0.8)
    area = map_label(c)
    x0, y0, x1, y1 = c.screen_box(area)
    cx, cy = (x0 + x1) / 2, (y0 + y1) / 2
    c.shot("map-open")
    r0 = row_of(area)
    c.check("the map starts at the first row of several", r0 and r0[0] == 1 and r0[1] > 3, str(r0))
    c.sb.scroll(3, cx, cy)
    r1 = row_of(map_label(c))
    c.check("the real wheel scrolls the map by rows", r1 and r1[0] > 1, str(r1))
    c.sb.scroll(-3, cx, cy)
    r2 = row_of(map_label(c))
    c.check("and back again", r2 and r2[0] == 1, str(r2))
    # Ctrl + wheel changes the zoom level
    c.sb._n("NotifyKeyboardKeysym", "(ub)", __import__("sandbox").keysym("Control"), True)
    try:
        c.sb.scroll(1, cx, cy)
    finally:
        c.sb._n("NotifyKeyboardKeysym", "(ub)", __import__("sandbox").keysym("Control"), False)
    c.check("Ctrl+wheel zooms out one level", c.wait(lambda: "Sections" in (map_label(c).name if map_label(c) else ""), 4))
    c.shot("map-zoomed")
    c.sb.move_to(cx, cy)
    time.sleep(0.4)
    c.check("the page under the pointer is named in the map", re.search(r"Page \d+", map_label(c).name or ""))
    # a drag through the book, then let go: the reader goes there
    before = c.page_label()
    c.sb.drag(x0 + 40, y0 + 40, x1 - 60, y1 - 60)
    c.check("letting go closes the map", c.wait(lambda: not map_label(c), 6))
    time.sleep(1.5)
    after = c.page_label()
    c.check("the reader went to the page under the pointer", before and after and after[0] > before[0] + 20,
            f"{before} -> {after}")


MAP_SCRIPT = (
    "wait:2500,read:1,wait:4000,"
    "reader:dwell=0.02|90,reader:dwell=0.021|40,reader:dwell=0.05|25,reader:dwell=0.052|120,reader:dwell=0.07|15,"
    "reader:dwell=0.1|60,reader:dwell=0.11|150,reader:dwell=0.115|30,reader:dwell=0.12|75,"
    "reader:hl-add=1|50|yellow,reader:hl-add=1|60|green,reader:hl-add=1|70|blue|underline,"
    "reader:hl-add=2|50|red|strikethrough|a note,reader:hl-add=3|50|purple,reader:hl-add=3|80|yellow,"
    "reader:hl-add=5|50|green|background|another note,reader:hl-add=8|50|blue,reader:hl-add=8|90|red,"
    "reader:hl-add=8|120|purple,reader:hl-add=8|150|yellow,"
    "reader:bookmark,wait:300,reader:map,wait:700,reader:map-click=0.08,wait:900,"
    "reader:bookmark,wait:300,reader:map,wait:700,reader:map-click=0.3,wait:900,"
    "reader:bookmark,wait:300,reader:map,wait:700,reader:map-click=0.14,wait:900"
)


def _look(c, name):
    big = os.path.join(c.dir, "big.epub")
    subprocess.run([sys.executable, os.path.join(HERE, "..", "make-big-epub.py"), big, "900"], check=True,
                   capture_output=True)
    c.books["big"] = big
    return big


def _open_map_with_marks(c):
    c.start(books=("big",), extra_env={"BOOKX_DEV_SCRIPT": MAP_SCRIPT})
    c.wait_node("slider", timeout=40)
    time.sleep(16)  # the dev script adds time, highlights and jumps
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    time.sleep(1.0)
    return area


@scenario
def book_map_look(c):
    """The map with highlight colours, notes, a bookmark, bars and numbered places; the card follows the pointer."""
    _look(c, "light")
    area = _open_map_with_marks(c)
    if not c.check("the map opens", area):
        return
    x0, y0, x1, y1 = c.screen_box(map_label(c))
    c.shot("1-overview")
    # rest the pointer on a block that has highlights: the card must be solid and inside the map
    c.sb.move_to(x0 + 14 + 22 * 6.5, y0 + 14 + 15 + 15)
    time.sleep(0.6)
    c.shot("2-hover")
    img = c.image()
    page = [n for n in c.find("label", rx=r"^Page \d+ · \d+%$")]
    if page:
        bx0, by0, bx1, by1 = c.screen_box(page[0])
        region = img.crop((int(bx1) - 70, int(by0) + 2, int(bx1) - 14, int(by1) - 2))
        px = list(region.getdata())
        spread = max(max(p[i] for p in px) - min(p[i] for p in px) for i in range(3))
        c.check("the preview card is solid: the map does not show through it", spread <= 4, f"spread {spread}")
    else:
        c.check("the preview card names the page", False)
    c.sb.move_to(x1 - 40, y0 + 14 + 15 + 15)
    time.sleep(0.6)
    c.shot("3-hover-right-edge")
    c.sb.move_to(x0 + 40, y1 - 50)
    time.sleep(0.6)
    c.shot("4-hover-bottom")
    c.sb.key("-")
    time.sleep(0.8)
    c.shot("5-zoomed-out")
    c.check("the look was captured (open the screenshots)", True)


@scenario
def book_map_look_dark(c):
    """The same in the dark style."""
    _look(c, "dark")
    c.start(books=("big",), extra_env={"BOOKX_DEV_SCRIPT": MAP_SCRIPT},
            settings="[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n")
    c.wait_node("slider", timeout=40)
    time.sleep(16)
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    time.sleep(1.0)
    if not c.check("the map opens", area):
        return
    x0, y0, x1, y1 = c.screen_box(map_label(c))
    c.shot("1-overview")
    c.sb.move_to(x0 + 14 + 22 * 6.5, y0 + 14 + 15 + 15)
    time.sleep(0.6)
    c.shot("2-hover")
    c.check("the look was captured (open the screenshots)", True)


@scenario
def book_map_look_contrast(c):
    """High contrast: thicker outlines."""
    _look(c, "hc")
    c.start(books=("big",), extra_env={"BOOKX_DEV_SCRIPT": MAP_SCRIPT},
            settings="[org/gnome/desktop/a11y/interface]\nhigh-contrast=true\n")
    c.wait_node("slider", timeout=40)
    time.sleep(16)
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    time.sleep(1.0)
    if not c.check("the map opens", area):
        return
    c.shot("1-overview")
    c.check("the look was captured (open the screenshot)", True)


def _median_lum(img, box):
    px = [0.2126 * r + 0.7152 * g + 0.0722 * b for r, g, b in img.crop(box).convert("RGB").getdata()]
    px.sort()
    return px[len(px) // 2]


def _map_theme(c, scheme, reader_theme):
    """The map, its card and its legend follow the app style (libadwaita light or dark), never the reading theme."""
    import json
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "config.json"), "w") as f:
        json.dump({"scheme": scheme, "reader": {"theme": reader_theme}}, f)
    _look(c, scheme)
    c.start(books=("big",), extra_env={"BOOKX_DEV_SCRIPT": MAP_SCRIPT})
    c.wait_node("slider", timeout=40)
    time.sleep(16)
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    time.sleep(1.0)
    if not c.check("the map opens", area):
        return
    dark_app = scheme == "Dark"
    x0, y0, x1, y1 = [int(v) for v in c.screen_box(map_label(c))]
    w, h = x1 - x0, y1 - y0
    # the dialog: the corner of the map area is the dialog's own background
    c.sb.move_to(x0 + w / 2, y1 + 40)
    time.sleep(0.5)
    img = c.shot_image = c.image(f"map-{scheme.lower()}-{reader_theme.lower()}-overview")
    bg = _median_lum(img, (x0 + 1, y0 + 1, x0 + 9, y0 + 9))
    if dark_app:
        c.check(f"app style {scheme}, reading theme {reader_theme}: the map is dark", bg < 90, f"brightness {bg}")
    else:
        c.check(f"app style {scheme}, reading theme {reader_theme}: the map is light", bg > 200, f"brightness {bg}")
    # hover in many places: edges, between blocks, over markers: the card is always solid, in the app's colours and inside
    cell, row_h, pad = 22.0, 61.0, 14.0
    # the cells per row and the rows in view follow the size of the dialog; the note of the test book is on page 49
    ncols = int((w - 2 * pad) / cell)
    rows = int((h - pad) / row_h)
    note_col, note_row = 49 % ncols, 49 // ncols
    spots = {
        "first block": (x0 + pad + cell * 0.5, y0 + pad + 15 + 15),
        "last block of a row": (x1 - 44 - pad - cell * 0.5, y0 + pad + 15 + 15),
        "between two blocks": (x0 + pad + cell * 5, y0 + pad + row_h + 30),
        "over the bookmark flag": (x0 + pad + cell * 0.5 + 6, y0 + pad + 15 - 2),
        "bottom row": (x0 + pad + cell * 9.5, y0 + pad + row_h * (rows - 1) + 30),
        "over the pencil of a note": (x0 + pad + cell * note_col - 3.5, y0 + pad + row_h * note_row + 15 + 30 + 9.5),
        "right edge": (x1 - 44 - pad - 2, y0 + pad + row_h * 3 + 30),
    }
    bad = []
    for name, (px, py) in spots.items():
        c.sb.move_to(px, py)
        time.sleep(0.8)
        n = map_label(c)
        m = re.search(r"card shown (\d+),(\d+),(\d+),(\d+)", n.acc.get_description() or "") if n else None
        if not m:
            bad.append(f"{name}: no card")
            continue
        cx, cy, cw_, ch_ = [int(v) for v in m.groups()]
        inside = cx >= 0 and cy >= 0 and cx + cw_ <= w and cy + ch_ <= h
        # the empty padding at the right edge of the card, in the screenshot
        wm = re.search(r"cardwin (\d+),(\d+),(\d+),(\d+)", n.acc.get_description() or "")
        ox, oy = 0, __import__("atspi").ORIGIN[1]  # the window fills the screen horizontally
        wx, wy, ww, wh = [int(v) for v in wm.groups()] if wm else (cx, cy, cw_, ch_)
        region = (int(ox) + wx + ww - 9, int(oy) + wy + 30, int(ox) + wx + ww - 4, int(oy) + wy + 70)
        verdicts = []
        for _ in range(3):  # a screenshot can catch a frame in the middle of a repaint
            img = c.image()
            lum = _median_lum(img, region)
            px_ = list(img.crop(region).convert("RGB").getdata())
            spread = max(max(p[i] for p in px_) - min(p[i] for p in px_) for i in range(3))
            right = lum < 120 if dark_app else lum > 200
            verdicts.append((inside and right and spread <= 4,
                             f"{name}: inside={inside} brightness={lum:.0f} spread={spread} card={[cx, cy, cw_, ch_]}"))
            time.sleep(0.3)
        if not all(v[0] for v in verdicts[1:]):
            bad.append(verdicts[-1][1])
            img.save(os.path.join(c.results, f"{c.name}-bad-" + name.replace(" ", "-") + ".png"))
    # the card of a page shows a thumbnail of the page in the reading theme; the note shows on its pencil
    c.sb.move_to(x0 + pad + cell * note_col - 3.5, y0 + pad + row_h * note_row + 15 + 30 + 9.5)
    time.sleep(1.5)
    c.check("the pencil of a note shows the note's text in the card", c.find("label", rx="^a note$"))
    c.sb.move_to(x0 + pad + cell * 6.5, y0 + pad + 15 + 15)
    n = None
    m = None
    for _ in range(15):
        time.sleep(0.5)
        n = map_label(c)
        m = re.search(r"thumb (\d+),(\d+),(\d+),(\d+) ready", n.acc.get_description() or "") if n else None
        if m:
            break
    if c.check("the card has a thumbnail of the page", m):
        tx, ty, tw, th = [int(v) for v in m.groups()]
        img = c.image(f"map-{scheme.lower()}-{reader_theme.lower()}-thumb")
        wm = re.search(r"thumbwin (\d+),(\d+),(\d+),(\d+)", n.acc.get_description() or "")
        ox, oy = 0, __import__("atspi").ORIGIN[1]
        if wm:
            tx, ty, tw, th = [int(v) for v in wm.groups()]
            box = (int(ox) + tx + 2, int(oy) + ty + 2, int(ox) + tx + tw - 2, int(oy) + ty + th - 2)
        else:
            box = (x0 + tx + 2, y0 + ty + 2, x0 + tx + tw - 2, y0 + ty + th - 2)
        px_ = list(img.crop(box).convert("RGB").getdata())
        med_px = sorted(px_, key=sum)[len(px_) // 2]
        ink = sum(1 for p in px_ if sum(abs(p[i] - med_px[i]) for i in range(3)) > 60)
        c.check("the thumbnail is 180 to 220 px wide, with the shape of the reader's page", 180 <= tw <= 220 and 0.4 < th / tw < 1.6, f"{tw}x{th}")
        c.check("it shows text, not a blank page", ink > 0.004 * len(px_), f"{ink} of {len(px_)} pixels differ from the page colour")
        med = _median_lum(img, box)
        reader_dark = reader_theme in ("Dark", "Black")
        c.check(f"the page in it uses the reading theme ({reader_theme}), the card around it the app style ({scheme})",
                (med < 110) == reader_dark, f"page brightness {med:.0f}")
    c.shot(f"map-{scheme.lower()}-{reader_theme.lower()}-hover")
    c.check("the card is opaque, inside the map and in the app's colours at every spot", not bad, "; ".join(bad))
    c.check("the app is alive", c.is_alive())


@scenario
def book_map_theme_light(c):
    _map_theme(c, "Light", "Dark")


@scenario
def book_map_theme_dark(c):
    _map_theme(c, "Dark", "Light")


@scenario
def book_map_theme_light_light(c):
    _map_theme(c, "Light", "Sepia")


# ---- the thumbnail is the reader's page (enhancement 18 follow-up) ----

def _desc(c):
    n = map_label(c)
    return (n.acc.get_description() or "") if n else ""


def _rect(d, key):
    m = re.search(key + r" (-?\d+),(-?\d+),(-?\d+),(-?\d+)", d)
    return tuple(int(v) for v in m.groups()) if m else None


def _grid(d):
    m = re.search(r"grid (\d+),(\d+),(\d+),(\d+),(\d+) pages (\d+)", d)
    return tuple(int(v) for v in m.groups()) if m else None


def _hash_dist(a, b):
    """Hamming distance of two 64 bit difference hashes (0 identical .. 64)."""
    from PIL import Image

    def h(im):
        g = im.convert("L").resize((9, 8), Image.LANCZOS)
        px = list(g.getdata())
        return [1 if px[y * 9 + x] > px[y * 9 + x + 1] else 0 for y in range(8) for x in range(8)]
    return sum(1 for x, y in zip(h(a), h(b)) if x != y)


def _mae(a, b, slack=6):
    """Mean absolute error (0..255) of two pictures after a light blur, `b` scaled to the size of `a`; the best of
    all shifts of up to `slack` pixels (the screenshot is cut out by window coordinates that can be a few pixels off)."""
    from PIL import Image, ImageFilter, ImageChops, ImageStat
    A = a.convert("L").filter(ImageFilter.GaussianBlur(1.2))
    B = b.convert("L").resize(A.size, Image.LANCZOS).filter(ImageFilter.GaussianBlur(1.2))
    w, h = A.size
    best = 255.0
    for dx in range(-slack, slack + 1):
        for dy in range(-slack, slack + 1):
            box_a = (max(0, dx), max(0, dy), w + min(0, dx), h + min(0, dy))
            box_b = (max(0, -dx), max(0, -dy), w + min(0, -dx), h + min(0, -dy))
            d = ImageChops.difference(A.crop(box_a), B.crop(box_b))
            best = min(best, ImageStat.Stat(d).mean[0])
    return best


def _cell_point(d, page):
    """Screen point of the middle of the block of `page` (nothing scrolled)."""
    cols, rows, first, ppc, aw, _total = _grid(d)
    ax, ay, aw_, ah = _rect(d, "areawin")
    oy = __import__("atspi").ORIGIN[1]
    cell = page // ppc
    row, col = divmod(cell, cols)
    cw = (aw - 28) / cols
    return (ax + 14 + (col + 0.5) * cw, oy + ay + 14 + (row - first) * 61.0 + 15 + 15)


def _wait_ready(c, timeout=12):
    """Polls the map's description until the thumbnail is ready. The node is found once and read again and again:
    walking the whole accessibility tree every quarter second would keep a debug build busy."""
    n = map_label(c)
    d = ""
    for i in range(int(timeout / 0.5)):
        time.sleep(0.5)
        if n is None or not d or i % 6 == 5:
            n = map_label(c) or n  # the node of a map that was just closed can still be around
        d = (n.acc.get_description() or "") if n else ""
        if " ready" in d and "thumbwin" in d:
            return d
    return d
    return d


def _faithful(c, scheme, reader_theme, book, title_rx, pages_frac=(0.18, 0.5, 0.82), max_mae=9.0, max_hash=20, control=True):
    """For three pages: jump there with the map, screenshot the reader's page, open the map again, hover the same
    block and compare the thumbnail with the reader's page scaled down (mean absolute error after a light blur and a
    difference hash); a page of another place is the control and must be clearly worse."""
    import json
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "config.json"), "w") as f:
        json.dump({"scheme": scheme, "reader": {"theme": reader_theme}}, f)
    if book == "big":
        _look(c, "faithful")
    c.start(books=(book,))
    c.open_book(title_rx=title_rx)
    time.sleep(2.5)
    oy = __import__("atspi").ORIGIN[1]
    results = []
    previous = None
    control_flag = control
    for k, frac in enumerate(pages_frac):
        c.sb.key("m")
        if not c.wait(lambda: map_label(c), 8):
            c.check("the map opens", False)
            return
        d = ""
        last, same = None, 0
        for _ in range(40):
            time.sleep(0.4)
            d = _desc(c)
            # the map's area is still settling (it moves while it opens): wait until it holds still
            rect = _rect(d, "areawin") if _grid(d) else None
            same = same + 1 if rect and rect == last else 0
            if same >= 3:
                break
            last = rect
        g = _grid(d)
        visible = g[0] * g[1] * g[3]
        page = max(1, min(g[5] - 1, int(min(visible, g[5]) * frac)))
        # aim from a fresh reading of the geometry; when the map moved meanwhile (it settles while it opens), aim again
        for _ in range(4):
            d = _desc(c) or d
            px, py = _cell_point(d, page)
            c.sb.move_to(px, py)
            time.sleep(0.5)
            m = re.search(r"cursor (\d+)", _desc(c))
            if m and int(m.group(1)) == page:
                break
        c.check(f"page {page}: the pointer is on its block before the click", re.search(r"cursor (\d+)", _desc(c)) and int(re.search(r"cursor (\d+)", _desc(c)).group(1)) == page,
                re.search(r"cursor (\d+)", _desc(c)).group(0) if re.search(r"cursor (\d+)", _desc(c)) else "")
        c.sb.click()  # goes to that page and closes the map
        c.check(f"the map closes when the block of page {page} is clicked", c.wait(lambda: not map_label(c), 6))
        time.sleep(1.8)
        reader_img = c.image()
        reader_img.save(os.path.join(c.results, f"{c.name}-p{page}-reader-full.png"))
        c.sb.key("m")
        c.wait(lambda: map_label(c), 8)
        d = ""
        for _ in range(20):
            time.sleep(0.4)
            d = _desc(c)
            if _grid(d):
                break
        px, py = _cell_point(d, page)
        c.sb.move_to(px - 2, py)
        time.sleep(0.2)
        c.sb.move_to(px, py)
        d = _wait_ready(c)
        cur = re.search(r"cursor (\d+) current (\d+)", d)
        c.note(f"page {page}: the pointer is on page {cur.group(1) if cur else None}, the reader is at map page {cur.group(2) if cur else None} (the start of its view)")
        thumb = _rect(d, "thumbwin")
        rr = _rect(d, "readerwin")
        col = re.search(r"thumbcol (\d)/(\d)", d)
        if not (thumb and rr):
            c.check(f"page {page}: the card has a thumbnail", False, d[-200:])
            c.sb.key("Escape")
            continue
        img = c.image(f"{c.name}-p{page}-map")
        tx, ty, tw, th = thumb
        t_img = img.crop((tx + 1, oy + ty + 1, tx + tw - 1, oy + ty + th - 1))
        rx, ry, rw, rh = rr
        ci, cn = (int(col.group(1)), int(col.group(2))) if col else (1, 1)
        pw = rw / cn
        r_img = reader_img.crop((int(rx + pw * (ci - 1)) + 1, int(oy + ry) + 1, int(rx + pw * ci) - 1, int(oy + ry + rh) - 1))
        mae = _mae(t_img, r_img)
        hd = _hash_dist(t_img, r_img)
        t_img.save(os.path.join(c.results, f"{c.name}-p{page}-thumb.png"))
        r_img.save(os.path.join(c.results, f"{c.name}-p{page}-reader.png"))
        control = _mae(t_img, previous) if previous is not None else None
        control_on = control_flag
        c.note(f"page {page}: thumb view {re.search(r'thumbview ([0-9,]+)', d).group(1) if re.search(r'thumbview ([0-9,]+)', d) else None}, reader view {re.search(r'readerview ([0-9,]+)', d).group(1) if re.search(r'readerview ([0-9,]+)', d) else None}")
        c.note(f"page {page}: MAE {mae:.1f}, dHash distance {hd}, control MAE {control}")
        results.append((page, mae, hd, control))
        c.note("desc: " + d[-330:])
        tvm = re.search(r"thumbview ([0-9,]+)", d)
        rvm = re.search(r"readerview ([0-9,]+)", d)
        c.check(f"page {page}: the thumbnail's page is the reader's page (chapter, first page of the view, pages: {tvm and tvm.group(1)} = {rvm and rvm.group(1)})",
                tvm and rvm and tvm.group(1) == rvm.group(1))
        c.check(f"page {page}: the thumbnail is the reader's page (mean abs error {mae:.1f} <= {max_mae}, dHash {hd} <= {max_hash})",
                mae <= max_mae and hd <= max_hash, f"mae {mae:.1f} hash {hd}")
        if control is not None and control_on:
            c.check(f"page {page}: a page of another place is clearly worse ({control:.1f} > {mae:.1f} + 3)", control > mae + 3,
                    f"control {control:.1f}")
        previous = r_img
        c.sb.key("Escape")
        time.sleep(0.6)
    c.check("three pages were compared", len(results) == 3, str(results))


@scenario
def book_map_faithful_light(c):
    _faithful(c, "Light", "Light", "big", "Big Book", control=False)


@scenario
def book_map_faithful_dark(c):
    _faithful(c, "Dark", "Dark", "big", "Big Book", control=False)


@scenario
def book_map_faithful_alice(c):
    _faithful(c, "Light", "Sepia", "alice", "Alice")


@scenario
def book_map_faithful_vertical(c):
    from scenarios_vertical import book as _book
    _book(c, "vsynth")
    _faithful(c, "Dark", "Dark", "vsynth", ".", pages_frac=(0.15, 0.5, 0.85))
