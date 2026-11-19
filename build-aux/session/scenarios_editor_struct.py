"""The editor's structure tools with real input: dragging entries in the Contents tab (CE-120), the Alt+arrow keys
there, merging two pages by dragging one onto the middle of another (CE-028), and the structure dialogs at two sizes
(cover, fonts, Transform HTML, jacket, accessibility metadata). Imported by scenarios.py."""
import re
import time

from harness import scenario
from scenarios_editor_tools import open_editor


def contents_tab(c):
    for n in c.find(None, "Contents"):
        if n.showing and n.role in ("toggle button", "radio button", "page tab", "tab", "button") and n.box[1] > 300:
            c.click_node(n)
            time.sleep(0.8)
            return True
    return False


def toc_entries(c):
    """The title entries of the Contents tab, top to bottom: (name, box)."""
    found = [n for n in c.find(None, "Contents entry title") if n.showing and n.box[0] < 300 and n.box[1] > 80]
    rows, seen = [], set()
    for n in sorted(found, key=lambda n: (n.box[1], n.box[2])):
        key = round(n.box[1] / 6)  # the entry and its text child both report the same row
        if key in seen:
            continue
        seen.add(key)
        rows.append(n)
    return rows


def left_edge(n):
    """Screen x of the left edge of a node (center() applies the window origin that box does not)."""
    return n.center()[0] - n.box[2] / 2


def top_edge(n):
    return n.center()[1] - n.box[3] / 2


def titles(entries):
    out = []
    for n in entries:
        try:
            out.append((n.get_text(), n.box[0]))
        except Exception:
            out.append((n.name, n.box[0]))
    return out


@scenario
def editor_toc_drag(c):
    """Drag the grip of a Contents entry: onto the middle of another makes it a child, above another puts it
    before; Alt+Left and Alt+Up move it back with the keyboard (CE-120)."""
    c.start(size="1280x800")
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    if not c.check("the Contents tab opens", contents_tab(c)):
        c.shot("no-contents-tab")
        return
    rows = toc_entries(c)
    before = titles(rows)
    c.note(f"entries: {before[:5]}")
    if not c.check("the Contents tab lists at least four entries", len(rows) >= 4, str(before)):
        c.shot("few-entries")
        return
    x0 = rows[0].box[0]
    # the grip sits 14 px left of the entry's title
    gx, gy = left_edge(rows[2]) - 14, rows[2].center()[1]
    tx, ty = rows[0].center()
    c.sb.drag(gx, gy, tx, ty)  # the middle of the first row: into it
    time.sleep(1.0)
    c.shot("toc-after-drop-into")
    after = titles(toc_entries(c))
    c.note(f"after the drop: {after[:5]}")
    names_before = [t for t, _ in before]
    c.check(
        "dropping the third entry onto the middle of the first makes it a child of the first",
        len(after) >= 3 and after[1][0] == names_before[2] and after[1][1] > x0 + 10,
        f"{after[:4]} from {before[:4]}",
    )
    # a drop on the upper edge of the first row puts the entry before it, at the top level
    rows = toc_entries(c)
    gx, gy = left_edge(rows[1]) - 14, rows[1].center()[1]
    c.sb.drag(gx, gy, left_edge(rows[0]) + 40, top_edge(rows[0]) + 2)
    time.sleep(1.0)
    c.shot("toc-after-drop-before")
    after2 = titles(toc_entries(c))
    c.note(f"after the second drop: {after2[:5]}")
    c.check(
        "dropping on the top edge of the first row puts the entry before it",
        len(after2) >= 2 and after2[0][0] == names_before[2],
        f"{after2[:4]}",
    )
    # the keyboard: select the first entry's title, Alt+Down moves it below its next sibling
    rows = toc_entries(c)
    first = after2[0][0]
    c.click_node(rows[0])
    time.sleep(0.4)
    c.sb.key("Down", ("Alt",))
    time.sleep(0.8)
    after3 = titles(toc_entries(c))
    c.note(f"after Alt+Down: {after3[:5]}")
    c.check(
        "Alt+Down moves the selected entry below the next one",
        len(after3) >= 2 and after3[0][0] != first and any(t == first for t, _ in after3[1:3]),
        f"{after3[:4]}",
    )
    c.sb.key("Right", ("Alt",))
    time.sleep(0.8)
    after4 = titles(toc_entries(c))
    c.note(f"after Alt+Right: {after4[:5]}")
    c.check(
        "the keys work again at once (the focus stays on the entry) and Alt+Right makes it a child",
        len(after4) >= 2 and any(t == first and x > x0 + 10 for t, x in after4[:4]),
        f"{after4[:4]}",
    )
    c.shot("toc-after-keys")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_file_drag_merge(c):
    """Drag a page onto the middle of another page: a question appears, and Merge joins them (CE-028)."""
    c.start(size="1280x800")
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")

    def chapter_labels():
        found = [n for n in c.find("label", rx=r"\.xhtml$") if n.showing and n.box[0] < 300 and n.box[1] > 80]
        return sorted(found, key=lambda n: n.box[1])

    rows = chapter_labels()
    if not c.check("the file list shows at least three pages", len(rows) >= 3, str([n.name for n in rows])):
        c.shot("no-rows")
        return
    names = [n.name for n in rows]
    (x0, y0), (x1, y1) = rows[2].center(), rows[0].center()
    c.sb.drag(x0, y0, x1, y1)  # the middle of the first page
    time.sleep(1.2)
    c.shot("merge-question")
    ask = [n for n in c.find(None, "Merge these pages?") if n.showing]
    if not c.check("dropping on the middle of a page asks whether to merge", ask, str(names[:4])):
        return
    btn = [n for n in c.find("button", "Merge") if n.showing]
    if c.check("the question has a Merge button", btn):
        c.click_node(btn[0])
        time.sleep(1.5)
        c.shot("after-merge")
        now = [n.name for n in chapter_labels()]
        c.note(f"pages now: {now[:5]} (before {names[:5]})")
        c.check("the dragged page is gone from the list", names[2] not in now and names[0] in now, str(now[:5]))
    c.check("the app is still alive", c.is_alive())


STRUCT_DIALOGS = ["Set Cover", "Manage Fonts", "Transform HTML", "Book Jacket", "Accessibility Metadata"]


def struct_smoke(c, size, tag):
    c.start(size=size)
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    opened = 0
    for title in STRUCT_DIALOGS:
        c.sb.key("p", ("Control", "Shift"))
        time.sleep(0.8)
        c.sb.type(title)
        time.sleep(0.6)
        c.sb.key("Return")
        time.sleep(1.4)
        dlg = [n for n in c.find(None, title) if n.showing]
        c.note(f"{title}: dialog nodes {[(n.role, n.box) for n in dlg][:3]}")
        if dlg:
            opened += 1
            c.shot(f"{tag}-{title.replace(' ', '-')}")
            win = c.find("frame")[0].box
            for b in c.find("button"):
                if b.showing and b.name in ("Apply", "Set Cover", "Preview") and b.box[2] > 20:
                    x, y, w, h = b.box
                    c.check(
                        f"{title}: the {b.name} button is inside the window",
                        0 <= x and x + w <= win[2] + 2 and 0 <= y and y + h <= win[3] + 2,
                        f"{b.box} in {win}",
                    )
                    break
            c.sb.key("Escape")
            time.sleep(0.6)
        else:
            c.shot(f"{tag}-{title.replace(' ', '-')}-none")
            c.sb.key("Escape")
            time.sleep(0.4)
    c.check("every structure dialog opens from the command palette", opened == len(STRUCT_DIALOGS), f"{opened} of {len(STRUCT_DIALOGS)}")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_struct_dialogs_1280(c):
    struct_smoke(c, "1280x800", "struct-1280")


@scenario
def editor_struct_dialogs_900(c):
    struct_smoke(c, "900x600", "struct-900")
