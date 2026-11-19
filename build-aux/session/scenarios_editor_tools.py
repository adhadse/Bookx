"""Editor tools with real input: the Reports window (a double-click on a row opens the file at that place) and
typing a hex code followed by Alt+X. Imported by scenarios.py. See docs/editor-spell-reports.md."""
import re
import time

from harness import scenario


def open_editor(c):
    card = c.wait_node("table cell", rx="Alice")
    c.sb.right_click(*card.center())
    c.wait_node("menu", timeout=5)
    items = c.find("menu item")
    c.edit_book_from_menu()  # Edit ▸ Edit Book
    ed = c.wait(lambda: [n for n in c.find("frame") if n.name.startswith("Edit Book")], 20)
    time.sleep(2)
    return ed


def subtitle(c):
    """The path shown under the book title in the editor's header."""
    for n in c.find("label"):
        if re.match(r"^OEBPS/[^ ]+$", n.name):
            return n.name
    return ""


def status_line(c):
    """(path, line) from the editor's status bar ("OEBPS/x.xhtml · Ln 16, Col 1 · 16 lines")."""
    for n in c.find("label"):
        m = re.match(r"^(\S+)\s+·\s+Ln (\d+), Col (\d+)", n.name)
        if m:
            return m.group(1), int(m.group(2))
    return None


class Reports:
    """Pointer access to the Reports window: its position on the screen is not known to AT-SPI (Wayland), so the
    origin is found by clicking the Links tab at a few likely places until the tab changes."""

    def __init__(self, c):
        self.c = c
        self.origin = None

    def node_center(self, n, origin):
        x, y, w, h = n.box
        return (origin[0] + x + w / 2, origin[1] + y + h / 2)

    def tab(self, name):
        for n in self.c.find(None, name):
            if n.role in ("radio button", "toggle button", "page tab", "tab") and n.showing and n.box[1] < 60:
                return n
        return None

    def calibrate(self):
        c = self.c
        links = self.tab("Links")
        if not links:
            return False
        for origin in [(149, 74), (150, 80), (150, 96), (143, 69), (157, 85), (150, 74), (149, 80)]:
            c.sb.click(*self.node_center(links, origin))
            if c.wait(lambda: c.find("label", "Source"), 1.5):
                self.origin = origin
                return True
        return False

    def click(self, n, count=1):
        self.c.sb.click(*self.node_center(n, self.origin), count=count)


@scenario
def editor_reports_double_click(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("r", ("Control", "Shift"))
    win = c.wait(lambda: [n for n in c.find("frame") if n.name == "Reports"], 10)
    if not c.check("Ctrl+Shift+R opens the Reports window", win):
        c.shot("no-reports")
        return
    time.sleep(1.5)
    rep = Reports(c)
    if not c.check("a real click on the Links tab shows the links table", rep.calibrate()):
        c.shot("reports-uncalibrated")
        return
    c.shot("reports-links")
    # links: double-click the Source of a row whose source file differs from the open one
    before = status_line(c)
    cells = [n for n in c.find("table cell", rx=r"^OEBPS/.+\.x?html?$") if n.showing and n.box[1] > 100]
    c.note(f"link rows: {[n.name for n in cells][:6]}")
    target = next((n for n in cells if before is None or n.name != before[0]), None)
    if c.check("the Links tab lists links with their source files", target is not None):
        rep.click(target, count=2)
        ok = c.wait(lambda: (status_line(c) or ("", 0))[0] == target.name, 6)
        now = status_line(c)
        c.check("a double-click on a link row opens its file in the editor", ok, f"{before} -> {now}")
        c.check("and puts the caret on the line of the link", now is not None and now[1] > 1, str(now))
        c.shot("after-link-jump")
    # files: double-click a chapter
    files = rep.tab("Files")
    if files:
        rep.click(files)
        time.sleep(0.8)
        chap = [n for n in c.find("table cell", rx=r"-h-3\.htm\.xhtml$") if n.showing]
        if c.check("the Files tab lists the chapters", chap):
            rep.click(chap[0], count=2)
            ok = c.wait(lambda: subtitle(c).endswith(chap[0].name), 6)
            c.check("a double-click on a file row opens that file", ok, subtitle(c))
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_alt_x(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type("<!-- zz")
    c.sb.type("00e9")
    c.sb.key("x", ("Alt",))
    time.sleep(0.6)
    text = code.get_text()
    c.check("typing a hex code and pressing Alt+X replaces it by the character", "zzé" in text and "zz00e9" not in text,
            text[-60:])
    c.sb.type(" U+2014")
    c.sb.key("x", ("Alt",))
    time.sleep(0.6)
    text = code.get_text()
    c.check("U+ codes work too and the prefix goes with it", "—" in text[-12:] and "U+2014" not in text, text[-60:])
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_file_drag_reorder(c):
    """Drag a chapter row onto another in the editor's file list: the reading order changes (CE-029)."""
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    def chapter_labels():
        found = [n for n in c.find("label", rx=r"\.xhtml$") if n.showing and n.box[0] < 300 and n.box[1] > 80]
        return sorted(found, key=lambda n: n.box[1])

    rows = chapter_labels()
    c.note(f"rows: {[(n.name, n.box) for n in rows][:6]}")
    if not c.check("the file list shows at least three pages", len(rows) >= 3, str([n.name for n in rows])):
        c.shot("no-rows")
        return
    moved = rows[2].name
    (x0, y0), (x1, y1) = rows[2].center(), rows[0].center()
    c.sb.drag(x0, y0, x1, y1 - 8)  # the upper half of the first page: drop before it
    time.sleep(1.0)
    c.shot("after-drag")
    now = [n.name for n in chapter_labels()]
    c.note(f"order now: {now[:5]}")
    c.check("dropping the third page above the first puts it first in the list", now and now[0] == moved, str(now[:4]))
    c.check("the app is still alive", c.is_alive())


# (group position in the Tools menu after the two one-click entries, entries, dialog titles)
TOOL_ENTRIES = [
    (0, 0, "Smarten Punctuation"), (0, 1, "Fix HTML"), (0, 2, "Beautify"),
    (1, 0, "Transform Styles"), (1, 1, "Remove Unused CSS Rules"), (1, 2, "Filter Style Information"),
    (2, 0, "Delete Unused Media"), (2, 1, "Embed Referenced Fonts"), (2, 2, "Download External Resources"),
    (3, 0, "Set Semantics"), (3, 1, "Accessibility"),
    (4, 0, "Check External Links"), (4, 1, "Repair Broken Links"), (4, 2, "Move Files to a Folder"),
]


def tools_smoke(c, size, tag):
    c.start(size=size)
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    opened = 0
    for _group, _entry, title in TOOL_ENTRIES:
        # the command palette lists every menu entry: type its name and press Enter
        c.sb.key("p", ("Control", "Shift"))
        time.sleep(0.8)
        c.sb.type(title)
        time.sleep(0.6)
        c.sb.key("Return")
        time.sleep(1.2)
        dlg = [n for n in c.find(None, title) if n.showing]
        c.note(f"{title}: dialog nodes {[(n.role, n.box) for n in dlg][:3]}")
        if dlg:
            opened += 1
            prev = [n for n in c.find("button", "Preview") if n.showing]
            if prev:
                c.click_node(prev[0])
                time.sleep(1.0)
            c.shot(f"{tag}-{title.replace(' ', '-')}")
            # nothing may be cut off: the dialog's buttons and text stay inside the window
            win = c.find("frame")[0].box
            for b in c.find("button"):
                if b.showing and b.name in ("Preview", "Apply") and b.box[2] > 20:
                    x, y, w, h = b.box
                    c.check(f"{title}: the {b.name} button is inside the window", 0 <= x and x + w <= win[2] + 2 and 0 <= y and y + h <= win[3] + 2, f"{b.box} in {win}")
                    break
            c.sb.key("Escape")
            time.sleep(0.6)
        else:
            c.shot(f"{tag}-{title.replace(' ', '-')}-none")
            c.sb.key("Escape")
            time.sleep(0.4)
    c.check("most tool dialogs open from the menu with real key presses", opened >= 9, f"{opened} of {len(TOOL_ENTRIES)}")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_tools_smoke_1280(c):
    tools_smoke(c, "1280x800", "1280")


@scenario
def editor_tools_smoke_900(c):
    tools_smoke(c, "900x600", "900")
