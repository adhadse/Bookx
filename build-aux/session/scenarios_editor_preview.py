"""Editor preview sync with real input: a click in the preview puts the caret at its element, and Split at
Cursor from the Markup menu splits the file at the caret. Imported by scenarios.py."""
import time

from harness import scenario
from scenarios_editor_tools import open_editor, status_line


def frame_box(c):
    fr = [n for n in c.find("frame") if n.name.startswith("Edit Book")]
    return fr[0].box if fr else None


@scenario
def editor_preview_click(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])  # the code view, not a search entry
    c.click_node(code)
    c.sb.key("Home", ("Control",))
    time.sleep(0.8)
    before = status_line(c)
    fx, fy, fw, fh = frame_box(c)
    cx, cy = code.center()
    # the preview is the right-hand pane: click in its text area, a few paragraphs down
    px = fx + fw - 180
    c.note(f"frame={frame_box(c)} code={code.box} click=({px}, {cy})")
    c.shot("preview-before")
    moved = False
    for dy in (0, 60, -60, 120):
        c.sb.click(px, cy + dy)
        time.sleep(0.6)
        now = status_line(c)
        if now and before and now[1] > before[1] + 2:
            moved = True
            break
    c.shot("preview-after-click")
    c.check("a real click in the preview puts the caret at that element in the code", moved, f"{before} -> {status_line(c)}")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_split_at_cursor(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    rows = [n for n in c.find(None, rx=r"^404022732.*htm\.xhtml") if n.showing]
    if c.check("the Files list shows chapters", len(rows) > 3):
        c.click_node(rows[3])
        time.sleep(1.5)
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("Home", ("Control",))
    for _ in range(31):
        c.sb.key("Down")
    time.sleep(0.5)
    c.note(f"caret at {status_line(c)}")
    btn = [n for n in c.find("toggle button") + c.find("button") if n.name.startswith("Markup:")]
    if not c.check("the Markup menu button exists", btn):
        return
    c.click_node(btn[0])
    # menu items have no accessible names here: the Structure submenu is the third entry of the first page and Split at
    # Cursor the third from the end of its page
    items = c.wait(lambda: [n for n in c.find("menu item") if n.showing and n.box[2] > 0] or None, 5)
    items = sorted(items or [], key=lambda n: n.box[1])
    c.note(f"first page rows: {len(items)}")
    if len(items) >= 3:
        c.click_node(items[2])
        time.sleep(0.8)
        items = sorted([n for n in c.find("menu item") if n.showing and n.box[2] > 0], key=lambda n: n.box[1])
    c.note(f"menu items by position: {[n.box[1] for n in items]}")
    c.shot("markup-menu")
    item = items[-3] if len(items) >= 10 else None
    if not c.check("the Markup menu offers Split at Cursor", item is not None):
        c.shot("no-split-item")
        return
    c.click_node(item)
    toast = c.wait(lambda: [n.name for n in c.find(None, rx=r"[Ss]plit") if "Split at" not in n.name], 4)
    c.note(f"toast: {toast}")
    ok = c.wait(lambda: c.find(None, rx=r"_split_001"), 8)
    c.shot("after-split")
    c.check("Split at Cursor adds the new file to the Files list", ok)
    c.check("the app is still alive", c.is_alive())


def markup_menu_pages(c, size):
    """Opens the Markup menu and each of its submenus; returns [(page, rows, lowest row bottom)] and the window box."""
    out = []
    btn = [n for n in c.find("toggle button") + c.find("button") if n.name.startswith("Markup:")]
    if not btn:
        return out, None
    fr = frame_box(c)
    for page in (None, 0, 1, 2):
        if page is None:
            c.click_node(btn[0])
        else:
            c.sb.key("Escape")
            time.sleep(0.4)
            c.click_node(btn[0])
        time.sleep(0.8)
        items = sorted([n for n in c.find("menu item") if n.showing and n.box[2] > 0], key=lambda n: n.box[1])
        if page is not None:
            if len(items) <= page:
                break
            c.click_node(items[page])
            time.sleep(0.8)
            items = sorted([n for n in c.find("menu item") if n.showing and n.box[2] > 0], key=lambda n: n.box[1])
        c.shot(f"markup-menu-{size}-{page}")
        if items:
            out.append((page, len(items), max(n.box[1] + n.box[3] for n in items), min(n.box[1] for n in items)))
    c.sb.key("Escape")
    return out, fr


def _fits(c, size):
    c.start(size=size)
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    pages, fr = markup_menu_pages(c, size)
    c.note(f"pages (page, rows, bottom, top): {pages} frame={fr}")
    if not c.check("the Markup menu and its submenus can be opened", len(pages) == 4, str(pages)):
        return
    top_rows = pages[0][1]
    c.check("the first page has the four entries and the three commands (Split Tag, Go to Definition, Sort CSS)", top_rows >= 7, str(pages[0]))
    fx, fy, fw, fh = fr
    for page, rows, bottom, top in pages:
        c.check(f"page {page} of the Markup menu fits in the window ({rows} rows, bottom {bottom} of {fy + fh}), no scrolling",
                bottom <= fy + fh - 2 and top >= fy, str((page, rows, bottom, top)))
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_markup_menu_fits_1280(c):
    _fits(c, "1280x800")


@scenario
def editor_markup_menu_fits_900(c):
    _fits(c, "900x600")


def region_diff(a, b, box):
    """Mean absolute difference of two screenshots inside box = (x0, y0, x1, y1)."""
    from PIL import ImageChops, ImageStat
    d = ImageChops.difference(a.crop(box), b.crop(box)).convert("L")
    return ImageStat.Stat(d).mean[0]


@scenario
def editor_scroll_sync_wheel(c):
    c.start()
    if not c.check("Edit Book… opens the editor window", open_editor(c)):
        return
    rows = [n for n in c.find(None, rx=r"^404022732.*htm\.xhtml") if n.showing]
    if c.check("the Files list shows chapters", len(rows) > 3):
        c.click_node(rows[3])
        time.sleep(2)
    fx, fy, fw, fh = frame_box(c)
    cx, cy = fx + 500, fy + 400  # inside the code view
    c.sb.click(cx, cy)
    c.sb.key("Home", ("Control",))
    time.sleep(1.0)
    code_box = (int(fx + 60), int(fy + 120), int(fx + fw - 380), int(fy + fh - 40))
    prev_box = (int(fx + fw - 350), int(fy + 120), int(fx + fw - 10), int(fy + fh - 40))
    top = c.image("sync-top")
    before = status_line(c)
    c.sb.scroll(18, cx, cy)  # the real wheel over the code view
    time.sleep(1.2)
    down = c.image("sync-scrolled")
    c.check("the wheel scrolled the code view", region_diff(top, down, code_box) > 4, f"{region_diff(top, down, code_box):.1f}")
    c.check("and the preview moved with it", region_diff(top, down, prev_box) > 4, f"{region_diff(top, down, prev_box):.1f}")
    c.check("without moving the caret", status_line(c) == before, f"{before} -> {status_line(c)}")
    c.sb.scroll(-18, cx, cy)
    time.sleep(1.2)
    back = c.image("sync-back")
    c.check("scrolling back up brings the preview back too", region_diff(top, back, prev_box) < 3, f"{region_diff(top, back, prev_box):.1f}")
    c.check("the app is still alive", c.is_alive())
