"""More screenshots for the public wiki: details and usage steps (build-aux/wiki-screenshots.sh). Same rules as
scenarios_wiki.py (leading "_" keeps them out of the normal run, WIKI_SCHEME=light|dark, no focus rings, no pointer);
pictures made with `crop=` are details (a widget with padding) and framed with a small shadow."""
import os
import time

from harness import scenario, Skip  # noqa: F401
from scenarios_wiki import no_rings, settings, wshot, shelf, set_config, maximize  # noqa: F401


def dump(c, hidden=False):
    """The accessibility tree into the scenario log (for finding names while writing a scene)."""
    c.note("TREE\n" + c.tree(hidden).dump())


def ui(c, rx=None, labels=False):
    """A compact list of the named, showing controls (role, name, screen box) in the scenario log."""
    keep = ("button", "toggle button", "check box", "radio button", "menu item", "text", "entry", "combo box", "label",
            "dialog", "list item", "page tab", "slider", "switch", "frame", "popup menu", "menu", "table cell",
            "spin button", "link", "tree item", "heading", "password text")
    import re as _re
    rows = []
    for n in c.tree().nodes:
        if not labels and n.role in ("label", "table cell"):
            continue
        if n.role in keep and n.name and (rx is None or _re.search(rx, n.name)) and n.box[2] > 0:
            rows.append(f"{n.role} {n.name[:70]!r} {c.screen_box(n)}")
    c.note("UI\n" + "\n".join(rows))


@scenario
def _probe(c):
    """Developer aid, not a wiki scene: runs the Python file named by $PROBE with `c` (and the helpers of this file) in a
    sandbox session. PROBE_SHELF=1 adds the shelf of generated books."""
    no_rings()
    books = ("alice", "tiny")
    if os.environ.get("PROBE_SHELF"):
        books = tuple(["alice"] + shelf(c))
    c.start(books=books, settings=settings(), extra_env={"BOOKX_TTS_MOCK": "300"})
    time.sleep(2)
    with open(os.environ["PROBE"]) as f:
        exec(compile(f.read(), os.environ["PROBE"], "exec"), globals() | {"c": c})


# ---- helpers -----------------------------------------------------------------------------------------------

def click(c, role=None, name=None, rx=None, timeout=6, pause=0.8):
    """Clicks a control found by role and name; None (and a note) when it is not there."""
    n = c.wait_node(role, name, rx, timeout=timeout)
    if not n:
        c.note(f"not found: {role} {name or rx}")
        return None
    c.click_node(n)
    time.sleep(pause)
    return n


def around(c, nodes, pad=14, **kw):
    """The union of the nodes' screen boxes, for wshot(crop=...)."""
    return [n for n in (nodes if isinstance(nodes, (list, tuple)) else [nodes]) if n is not None]


def lib_start(c, script=None, cfg=None, books=True, size="1280x800", env=None):
    """The library with a shelf of generated books (no real library involved)."""
    names = shelf(c) if books else []
    if cfg:
        set_config(**cfg)
    no_rings()
    e = {"BOOKX_TTS_MOCK": "300"}
    if script:
        e["BOOKX_DEV_SCRIPT"] = script
    e.update(env or {})
    c.start(books=tuple(["alice"] + names) if books else (), settings=settings(), size=size, extra_env=e)
    if books:
        c.wait_node("table cell", rx="Alice")
    time.sleep(2.5)


def close(c):
    c.sb.key("Escape")
    time.sleep(0.6)


# ---- library: small details --------------------------------------------------------------------------------

@scenario
def _wiki_lib_steps(c):
    """Search bar, sidebar, view options, main menu, book menu, new shelf, switch library."""
    lib_start(c)
    c.sb.key("f", ("Control",))
    time.sleep(0.8)
    c.sb.type("author:melville OR author:austen")
    time.sleep(1.5)
    wshot(c, "library-search", crop=(256, 37, 1280, 460), pad=0)
    c.sb.key("a", ("Control",))
    c.sb.key("BackSpace")
    close(c)
    wshot(c, "library-sidebar", crop=(7, 37, 252, 800), pad=0)
    click(c, "toggle button", "View options")
    wshot(c, "library-view-options", crop=(972, 80, 1280, 318), pad=0)
    close(c)
    click(c, "toggle button", "Main Menu")
    items = c.find("button", rx="^(Open Library Folder|About Bookx|Vocabulary)$")
    wshot(c, "library-main-menu", crop=items, pad=22)
    close(c)
    card = c.wait_node("table cell", rx="Moby")
    c.sb.click(*card.center())
    time.sleep(1.5)
    card = c.wait_node("table cell", rx="Moby")
    c.sb.right_click(*card.center())
    time.sleep(1.0)
    close(c)
    time.sleep(0.8)
    before = snap(c)
    c.sb.right_click(*card.center())
    time.sleep(1.2)
    wshot(c, "library-book-menu", crop=popup_box(before, snap(c)), pad=6)
    close(c)
    before = snap(c)
    click(c, "button", "New Shelf")
    n = c.wait_node("text", "Shelf name")
    if n:
        c.sb.type("To read")
        time.sleep(0.6)
        wshot(c, "library-new-shelf", crop=dialog_box(before, snap(c)), pad=-6)
    close(c)
    click(c, "toggle button", "Switch library")
    wshot(c, "library-switch", crop=(0, 76, 360, 250), pad=0)
    c.check("details taken", True)


# ---- finding popovers and dialogs by what they change on the screen ----------------------------------------

def snap(c):
    """The screen now (without the pointer), to compare with after a popover or a dialog opened."""
    c.sb.move_to(c.sb.w // 2, 10)
    time.sleep(0.4)
    return c.image()


def popup_box(before, after, thr=20, region=None):
    """The rectangle a popover or menu added to the screen: where `after` differs from `before` (the top bar is ignored)."""
    from PIL import ImageChops
    d = ImageChops.difference(before, after).convert("L").point(lambda v: 255 if v > thr else 0)
    x0, y0, x1, y1 = region or (0, 36, d.width, d.height)
    b = d.crop((x0, y0, x1, y1)).getbbox()
    return None if not b else (b[0] + x0, b[1] + y0, b[2] + x0, b[3] + y0)


def dialog_box(before, after, thr=5):
    """The rectangle of a modal dialog: the page behind it is dimmed by a constant factor, so the pixels that are not
    `before` times that factor belong to the dialog."""
    from PIL import ImageChops
    w, h = before.size
    ratios = {}
    for y in range(60, h - 20, 23):
        for x in range(10, w - 10, 29):
            b, a = before.getpixel((x, y)), after.getpixel((x, y))
            if sum(b) > 300:
                r = round(sum(a) / sum(b), 2)
                ratios[r] = ratios.get(r, 0) + 1
    k = max(ratios, key=ratios.get) if ratios else 0.8
    expect = before.point(lambda v: int(v * k))
    d = ImageChops.difference(expect, after).convert("L").point(lambda v: 255 if v > thr else 0)
    d = d.crop((0, 36, w, h)).filter(__import__("PIL.ImageFilter", fromlist=["x"]).MinFilter(5))
    b = d.getbbox()
    return None if not b else (b[0] - 2, b[1] + 34, b[2] + 2, b[3] + 38)


def main_menu(c, name, wait=1.5):
    """Opens the main menu and presses an entry."""
    if not click(c, "toggle button", "Main Menu", pause=0.8):
        return None
    return click(c, "button", name, pause=wait)


def dialog_shot(c, label, opener, pad=-6, wait=1.5, **kw):
    """Runs `opener`, then takes a detail picture of the dialog it opened (found by what it dims)."""
    before = snap(c)
    opener()
    time.sleep(wait)
    box = dialog_box(before, snap(c))
    for _ in range(3):  # the dialog may still be opening: a box that is only a sliver is tried again
        if box is not None and box[3] - box[1] > 100:
            break
        time.sleep(1.2)
        box = dialog_box(before, snap(c))
    if box is None or box[3] - box[1] <= 100:
        box = None
    if box is None:
        c.note(f"{label}: no dialog found")
        return None
    c.note(f"{label}: dialog box {box}")
    return wshot(c, label, crop=box, pad=pad, **kw)


@scenario
def _wiki_lib_dialogs(c):
    """The dialogs of the main menu, one detail picture each."""
    lib_start(c)
    for name, label in [("Reading Statistics", "library-statistics"), ("Reading Trackers", "library-trackers"), ("Audiobooks", "library-audiobooks"),
                        ("Import Highlights", "library-import-highlights"),
                        ("Export All Annotations", "library-export-annotations"),
                        ("Export Citations", "library-citations"), ("Sync", "library-share"),
                        ("Keyboard Shortcuts", "shortcuts-window"),
                        ("Saved Searches", "library-saved-searches"), ("Online Catalogues", "library-catalogues")]:
        dialog_shot(c, label, lambda: main_menu(c, name))
        ui(c)
        close(c)
        if not c.is_alive():
            break
    c.check("dialogs taken", True)


# ---- reader ------------------------------------------------------------------------------------------------

READ = "wait:2500,read:1,wait:4000,reader:goto=2,wait:2500"


def reader_start(c, script=READ, cfg=None, env=None, books=("alice",), settle=5, extra=""):
    """A sandbox with Alice open at a page of text (a dev script does the opening, the pictures use real input)."""
    if cfg:
        set_config(**cfg)
    no_rings()
    e = {"BOOKX_TTS_MOCK": "600", "BOOKX_DEV_SCRIPT": script + extra}
    e.update(env or {})
    c.start(books=books, settings=settings(), extra_env=e)
    if not c.wait_node("slider", timeout=25):
        raise RuntimeError("the book did not open (the screenshot run tries the scene again)")
    time.sleep(settle)


def popover_shot(c, label, opener, pad=8, wait=1.0, **kw):
    """Runs `opener` (a click that opens a popover or a panel) and takes a detail picture of what appeared."""
    before = snap(c)
    opener()
    time.sleep(wait)
    box = popup_box(before, snap(c))
    if box is None:
        c.note(f"{label}: nothing appeared")
        return None
    c.note(f"{label}: box {box}")
    return wshot(c, label, crop=box, pad=pad, **kw)


@scenario
def _wiki_reader_steps(c):
    """Reader popovers and panels: text appearance, menu, search, ruler, bookmark."""
    reader_start(c)
    popover_shot(c, "reader-text-settings", lambda: click(c, "toggle button", "Text Appearance"))
    ui(c)
    close(c)
    popover_shot(c, "reader-menu", lambda: click(c, "toggle button", "Main Menu"))
    close(c)
    c.sb.key("f", ("Control",))
    time.sleep(0.6)
    c.sb.type("rabbit")
    time.sleep(2.0)
    wshot(c, "reader-search", crop=(830, 36, 1280, 600), pad=0)
    c.check("steps taken", True)


@scenario
def _wiki_reader_words(c):
    """A selected word: the dictionary result, the vocabulary list it fills, the highlight colours of the popover."""
    from scenarios import select_word
    reader_start(c)
    before = snap(c)
    select_word(c)
    btn = c.wait_node("button", "Look Up in Dictionary", timeout=6)
    if btn:
        time.sleep(0.8)
        wshot(c, "reader-selection-popover", crop=popup_box(before, snap(c)), pad=10)
        before = snap(c)
        c.click_node(btn)
        c.wait_node("label", rx="Test definition of ", timeout=6)
        time.sleep(1.2)
        box = popup_box(before, snap(c))
        wshot(c, "reader-dictionary", crop=box, pad=8)
    close(c)
    close(c)
    c.sb.key("Escape")
    time.sleep(0.5)
    c.check("words taken", True)


@scenario
def _wiki_reader_cards(c):
    """Speed reading card, read-aloud card, goto dialog, bookmarks."""
    reader_start(c)
    click(c, "toggle button", "Main Menu")
    before = snap(c)
    # GTK gives the entries of the gio::Menu popover no accessible names: Speed Reading is the second one
    items = c.wait(lambda: sorted([n for n in c.find("menu item") if n.showing and n.box[2] > 0],
                                  key=lambda n: n.box[1]) or None, 5)
    item = items[1] if items and len(items) > 1 else None
    if item:
        c.click_node(item)
        time.sleep(1.2)
        start = c.wait_node("button", "Start", timeout=6)
        if start:
            c.click_node(start)
            time.sleep(1.5)
            c.sb.key("space")
            time.sleep(0.6)
            ui(c)
            wshot(c, "reader-speed-card", crop=c.find("button", rx="^(Faster|Slower|Close speed reading|Start|Pause)$"),
                  pad=40, minsize=(520, 200))
        c.sb.key("Escape")
        time.sleep(0.6)
    dialog_shot(c, "reader-goto", lambda: c.sb.key("g", ("Control",)), wait=1.0)
    close(c)
    c.check("cards taken", True)


# ---- library: panels, banner, shelves, health, preferences -------------------------------------------------

LASTMANY = "wait:2500,library:lastread-many"


@scenario
def _wiki_lib_banner(c):
    """The Continue Reading banner: the hero card with the 'Also reading' chips (tall window), then the slim strip."""
    lib_start(c, script=LASTMANY + ",library:banner-mode=large", size="1500x1080")
    maximize(c)
    time.sleep(3)
    wshot(c, "library-banner-also", crop=(256, 80, 1500, 420), pad=0)
    c.check("banner shot", True)


@scenario
def _wiki_lib_banner_strip(c):
    lib_start(c, script=LASTMANY + ",library:banner-mode=compact")
    time.sleep(2)
    wshot(c, "library-banner-chips", crop=(256, 37, 1280, 200), pad=0)
    c.check("strip shot", True)


@scenario
def _wiki_lib_shelves(c):
    """Shelves in the sidebar and a shelf's books."""
    lib_start(c, script="wait:2500,library:shelf=create=To read,library:shelf=create=Classics,"
                        "library:shelf=add=To read|2+3+7,library:shelf=add=Classics|1+5+8+9")
    time.sleep(2)
    wshot(c, "library-shelves", crop=(7, 37, 252, 420), pad=0)
    click(c, None, rx="^Classics")
    time.sleep(1.5)
    wshot(c, "library-shelf-open", crop=(256, 37, 1280, 460), pad=0)
    c.check("shelves shot", True)


@scenario
def _wiki_lib_jobs(c):
    """The Background Jobs panel with a running, a finished and a failed job."""
    lib_start(c, script="wait:2500,jobs:start=import|Importing 12 books|cancel,jobs:progress=1|0.45|Moby-Dick.epub,"
                        "jobs:start=export|Exporting the library as Calibre|cancel,jobs:progress=2|0.8|,"
                        "jobs:start=send|Sending to Boox|,jobs:finish=3|The device is not mounted,"
                        "jobs:start=fetch|Fetching metadata for 40 books|cancel,jobs:progress=4|0.2|Dracula")
    time.sleep(2)
    popover_shot(c, "library-jobs-panel", lambda: click(c, "toggle button", "Background Jobs"), pad=6)
    c.check("jobs shot", True)


def health_tab(c, name, label):
    n = c.wait_node(None, rx=f"^{name}$", timeout=4)
    if n:
        c.click_node(n)
        time.sleep(1.0)
        wshot(c, label, crop=c.hbox, pad=-6)


@scenario
def _wiki_lib_health(c):
    """The tabs of Library Health."""
    lib_start(c)
    before = snap(c)
    click(c, "list item", "Library Health", pause=1.2)
    c.hbox = dialog_box(before, snap(c))
    wshot(c, "library-health-checks", crop=c.hbox, pad=-6)
    ui(c)
    for name, label in [("Duplicates", "library-health-duplicates"), ("Editions", "library-health-editions"),
                        ("Needs Attention", "library-health-attention"), ("Backups", "library-health-backups"),
                        ("Trash", "library-health-trash")]:
        health_tab(c, name, label)
        if name == "Backups":
            ui(c, labels=True)
            c.check("the Backups panel explains the catalogue-only scope before a run",
                    c.find(None, "What is in a backup"))
    c.check("health shot", True)


@scenario
def _wiki_lib_empty(c):
    """The welcome page of an empty library and the Add menu."""
    lib_start(c, books=False)
    time.sleep(1.5)
    wshot(c, "library-welcome", crop=(256, 37, 1280, 800), pad=0)
    popover_shot(c, "library-add-menu", lambda: click(c, "toggle button", rx="^Add Books"))
    ui(c)
    c.check("empty shot", True)


PREFS_PAGES = [("General", "general"), ("Library", "library"), ("Reading", "reading"), ("Sync", "sync"),
               ("Editor", "editor"), ("E-mail", "email"), ("Extensions", "extensions"),
               ("Text to Speech", "tts"), ("Keyboard Shortcuts", "shortcuts"),
               ("Privacy", "privacy"), ("AI Assistants", "mcp")]


@scenario
def _wiki_prefs_pages(c):
    """Every page of Preferences, one picture each."""
    lib_start(c)
    before = snap(c)
    c.sb.key(",", ("Control",))
    time.sleep(1.5)
    box = dialog_box(before, snap(c))
    c.note(f"preferences box {box}")
    for name, slug in PREFS_PAGES:
        n = c.wait_node("list item", name, timeout=3)
        if not n:
            c.note(f"no page {name}")
            continue
        c.click_node(n)
        time.sleep(1.5)
        c.sb.move_to(c.sb.w // 2, 10)
        wshot(c, f"preferences-{slug}", crop=box, pad=-6)
        if slug == "extensions":
            # Patches is a section of the same page now (visible below Plugins), so it gets its own picture of it
            wshot(c, "preferences-patches", crop=box, pad=-6)
    c.check("preferences shot", True)


# ---- editor ------------------------------------------------------------------------------------------------

def editor_start(c):
    from scenarios_editor_tools import open_editor
    no_rings()
    c.start(books=("alice",), settings=settings(), extra_env={"BOOKX_TTS_MOCK": "300"})
    if not open_editor(c):
        raise RuntimeError("the editor did not open")
    time.sleep(2)


def palette_run(c, query, wait=1.8):
    """Runs an editor command through the command palette (Ctrl+Shift+P)."""
    c.sb.key("p", ("Control", "Shift"))
    time.sleep(0.8)
    c.sb.type(query)
    time.sleep(0.8)
    c.sb.key("Return")
    time.sleep(wait)


def open_file(c, name):
    """Opens a file of the book with Open File (Ctrl+P) by a part of its name."""
    c.sb.key("p", ("Control",))
    time.sleep(0.8)
    c.sb.type(name)
    time.sleep(0.8)
    c.sb.key("Return")
    time.sleep(1.5)


@scenario
def _wiki_ed_code(c):
    """The code view in use: misspelling squiggles, the completion list, colour swatches in CSS."""
    editor_start(c)
    open_file(c, "h-3.htm")
    code = c.wait_node("text", "Book source editor")
    c.click_node(code)
    c.sb.key("Home", ("Control",))
    for _ in range(40):
        c.sb.key("Down")
    time.sleep(1.2)
    x0, y0, x1, y1 = c.screen_box(code)
    wshot(c, "editor-code", crop=(x0, y0, x1, y0 + 330), pad=0)
    c.sb.key("Home", ("Control",))
    for _ in range(12):
        c.sb.key("Down")
    c.sb.key("End")
    c.sb.key("Return")
    c.sb.type("<di")
    time.sleep(1.5)
    x0, y0, x1, y1 = c.screen_box(c.wait_node("text", "Book source editor"))
    wshot(c, "editor-completion", crop=(x0, y0, x1, y1), pad=0)
    c.sb.key("Escape")
    open_file(c, "pgepub.css")
    c.click_node(c.wait_node("text", "Book source editor"))
    c.sb.key("Home", ("Control",))
    c.sb.type("p.note { color: #c0392b; background: #fdf2c4; border-left: 4px solid #2e7d32; }\n\n")
    time.sleep(1.2)
    code = c.wait_node("text", "Book source editor")
    x0, y0, x1, y1 = c.screen_box(code)
    wshot(c, "editor-swatches", crop=(x0, y0, x1, y0 + 260), pad=0)
    c.check("code shots", True)


@scenario
def _wiki_ed_menus(c):
    """Tools, Markup and Book menus, the problems panel, the Contents and Metadata tabs."""
    editor_start(c)
    def book_menu():
        # the library window behind the editor has its own "Main Menu" button: the editor's is the right-most
        btns = c.find("toggle button", "Main Menu")
        if btns:
            c.click_node(max(btns, key=lambda n: n.center()[0]))
            time.sleep(0.8)

    for rx, label, box in [("^Tools", "editor-tools-menu", (1000, 76, 1280, 800)),
                           ("^Markup", "editor-markup-menu", (900, 76, 1280, 340)),
                           ("^Main Menu", "editor-book-menu", None)]:
        if box:
            click(c, "toggle button", rx=rx)
            wshot(c, label, crop=box, pad=0)
        else:
            book_menu()  # the popup box detection also picks up the preview pane here: fixed crop
            wshot(c, label, crop=(960, 76, 1280, 556), pad=0)
        close(c)
    open_file(c, "h-3.htm")
    c.click_node(c.wait_node("text", "Book source editor"))
    c.sb.key("Home", ("Control",))
    for _ in range(26):
        c.sb.key("Down")
    c.sb.type("<p>A <b>bold start and <i>nested</b> wrong</i> with <img src=\"missing.png\"/></p>\n")
    c.sb.key("s", ("Control",))
    time.sleep(1.0)
    c.sb.key("F7")
    time.sleep(3.5)
    wshot(c, "editor-problems", crop=(7, 430, 1280, 800), pad=0)
    for tab, label in [("Contents", "editor-contents-tab"), ("Metadata", "editor-metadata-tab")]:
        click(c, "page tab", tab, pause=1.2)
        wshot(c, label, crop=(7, 80, 262, 800), pad=0)
    c.check("menus shot", True)


@scenario
def _wiki_ed_dialogs(c):
    """Dialogs of the editor, opened through its command palette."""
    editor_start(c)
    for q, label in [("Add Narration", "editor-narration"), ("Manage Snippets", "editor-snippets"),
                     ("Set Cover", "editor-set-cover"),
                     ("Manage Fonts", "editor-manage-fonts"),
                     ("Transform HTML", "editor-transform-html"), ("Book Jacket", "editor-jacket"),
                     ("Accessibility Metadata", "editor-a11y-form"), ("Insert Hyperlink", "editor-insert-link"),
                     ]:
        dialog_shot(c, label, lambda: palette_run(c, q), wait=2.0)
        for _ in range(3):
            close(c)
        time.sleep(0.8)
    c.check("editor dialogs", True)


@scenario
def _wiki_ed_dialogs2(c):
    """More dialogs of the editor (a second session: the first one is long)."""
    editor_start(c)
    for q, label in [("Rename Tag", "editor-rename-tag"), ("Editor Preferences", "editor-preferences"),
                     ("Checkpoints", "editor-checkpoints"), ("Bulk Rename", "editor-bulk-rename")]:
        dialog_shot(c, label, lambda: palette_run(c, q), wait=2.5)
        for _ in range(3):
            close(c)
        time.sleep(1.0)
    c.check("editor dialogs 2", True)


@scenario
def _wiki_ed_panels(c):
    """Panels that are not modal: spelling, special characters."""
    editor_start(c)
    open_file(c, "h-3.htm")
    palette_run(c, "Check Spelling", wait=3.0)
    wshot(c, "editor-spelling", crop=(921, 125, 1280, 704), pad=0)
    before = snap(c)
    palette_run(c, "Insert Special Character", wait=1.5)
    wshot(c, "editor-special-chars", crop=popup_box(before, snap(c), region=(250, 40, 1100, 790)), pad=4)
    c.check("panels shot", True)


# ---- reader: themes, notes, read aloud, map, vocabulary, formats -------------------------------------------

THEME_X = {"light": 1071, "sepia": 1121, "dark": 1171}


def _theme_scene(name, key):
    def scene(c):
        reader_start(c)
        click(c, "toggle button", "Text Appearance")
        click(c, "toggle button", key.capitalize())
        time.sleep(1.0)
        close(c)
        time.sleep(1.5)
        wshot(c, f"reader-theme-{key}", crop=(120, 90, 1160, 700), pad=0)
        c.check("theme shot", True)
    scene.__name__ = name
    return scenario(scene)


for _key in ("light", "sepia", "dark"):
    _theme_scene(f"_wiki_reader_theme_{_key}", _key)


@scenario
def _wiki_reader_notes(c):
    """The highlight popover with a note (colours, styles) and the notes list in the sidebar."""
    reader_start(c, extra=",reader:demo-highlights,wait:1500,reader:ui-note-popover", settle=6)
    wiki = c.wait_node("button", "Look Up on Wikipedia", timeout=12)
    c.sb.move_to(c.sb.w // 2, 10)
    time.sleep(0.6)
    nodes = [n for n in c.find("button") + c.find("toggle button") if n.box[1] > 150 and n.box[1] < 700
             and n.box[0] > 100] + c.find(None, rx="Where the story really starts")
    wshot(c, "reader-note-popover", crop=nodes, pad=22, minsize=(420, 200))
    c.check("note popover", wiki)


@scenario
def _wiki_reader_aloud(c):
    """The read-aloud popover and the player card (a silent mock engine)."""
    reader_start(c)
    btn = c.wait_node("toggle button", rx="^Read Aloud")
    popover_shot(c, "read-aloud-menu", lambda: c.click_node(btn), pad=6)
    play = c.wait_node("button", "Read Aloud from Here", timeout=5)
    if play:
        c.click_node(play)
        c.sb.key("Escape")
        c.wait_node("button", "Next Sentence", timeout=6)
        time.sleep(2.0)
        ui(c)
        card = c.find("button", rx="^(Next Sentence|Previous Sentence|Close Read Aloud|Pause reading|Next Chapter|Previous Chapter|Faster|Slower)")
        wshot(c, "read-aloud-card", crop=card, pad=36, minsize=(540, 160))
    c.check("aloud shot", play)


@scenario
def _wiki_reader_vocab(c):
    """The vocabulary list after looking a word up (the library's Vocabulary window)."""
    from scenarios import select_word
    reader_start(c)
    select_word(c)
    btn = c.wait_node("button", "Look Up in Dictionary", timeout=6)
    if btn:
        c.click_node(btn)
        c.wait_node("label", rx="Test definition of ", timeout=6)
        time.sleep(1.0)
    c.sb.key("Escape")
    time.sleep(0.5)
    c.sb.key("Escape")
    time.sleep(0.5)

    def open_vocab():
        c.sb.key("k", ("Control",))
        time.sleep(0.8)
        c.sb.type("Vocabulary")
        time.sleep(0.8)
        c.sb.key("Return")
    dialog_shot(c, "library-vocabulary-words", open_vocab, wait=2.0)
    ui(c)
    c.check("vocab shot", True)


@scenario
def _wiki_reader_pdf(c):
    """A PDF in the reader."""
    if not c.books.get("pdf"):
        raise Skip("no test PDF")
    no_rings()
    c.start(books=("pdf",), settings=settings())
    c.open_book("")
    time.sleep(3)
    wshot(c, "reader-pdf")
    c.check("pdf shot", True)


@scenario
def _wiki_reader_comic(c):
    """The comic reader: one page, the two-page layout, a manga read right to left."""
    if not c.books.get("comic"):
        raise Skip("no test comic")
    reader_start(c, script="wait:2500,read:1,wait:4000,reader:comic-goto=2,wait:1500", books=("comic",))
    wshot(c, "reader-comic-page")
    c.check("comic shot", True)


# ---- library: details, metadata, device, news --------------------------------------------------------------

@scenario
def _wiki_lib_details(c):
    """The details panel, Edit Metadata, the authors manager, the Convert menu."""
    lib_start(c, script="wait:2500,library:lastread")
    card = c.wait_node("table cell", rx="Moby")
    c.sb.click(*card.center())
    time.sleep(2.0)
    wshot(c, "library-details-panel", crop=(936, 76, 1280, 800), pad=0)
    dialog_shot(c, "library-edit-metadata", lambda: click(c, "button", "Edit Metadata"), wait=2.0)
    ui(c)
    for _ in range(2):
        close(c)
    dialog_shot(c, "library-authors", lambda: click(c, "button", "Manage authors"))
    close(c)
    close(c)
    c.check("details shot", True)


@scenario
def _wiki_lib_menus_misc(c):
    """Add from the Web, News Subscriptions (from the Add menu)."""
    lib_start(c)
    for q, label in [("Add from the Web", "library-add-web"), ("News Subscriptions", "library-news")]:
        def pick():
            c.sb.key("k", ("Control",))
            time.sleep(0.8)
            c.sb.type(q)
            time.sleep(0.8)
            c.sb.key("Return")
        dialog_shot(c, label, pick, wait=2.0)
        for _ in range(3):
            close(c)
    c.check("misc shot", True)


@scenario
def _wiki_lib_device(c):
    """The connected banner and both pages of the Devices window with a mounted e-reader (a fake Boox folder)."""
    import shutil
    mounts = os.path.join(c.dir, "mounts")
    books = os.path.join(mounts, "Boox", "Books")
    os.makedirs(books, exist_ok=True)
    shutil.copy(c.books["tiny"], os.path.join(books, "Tiny Book.epub"))
    shutil.copy(c.books["alice"], os.path.join(books, "Alice in Wonderland.epub"))
    lib_start(c)
    time.sleep(1.5)
    banner = (c.find(None, "E-reader connected") + c.find("button", "Sync")
              + c.find("button", rx="^E-reader"))
    if banner:
        wshot(c, "library-device-banner", crop=banner, pad=40, minsize=(900, 120))
    dev = c.wait_node("button", rx="^E-reader", timeout=6)
    ui(c)
    if dev:
        c.click_node(dev)
        time.sleep(2.5)
        ui(c)
        wshot(c, "device-window-detail")
        settings = (c.find("page tab", "Settings") or c.find("toggle button", "Settings")
                    or c.find("button", "Settings") or c.find(None, "Settings"))
        if settings:
            c.click_node(settings[0])
            time.sleep(1.2)
            ui(c)
            wshot(c, "device-window-settings")
        else:
            c.note("no Settings page: the Devices Settings picture was not shot")
    c.check("device shot", dev)


@scenario
def _wiki_prefs_details(c):
    """Details of Preferences: the offline switch, a shortcut being changed, the continue-reading setting."""
    lib_start(c)
    before = snap(c)
    c.sb.key(",", ("Control",))
    time.sleep(1.5)
    pbox = dialog_box(before, snap(c))
    click(c, "list item", "Privacy", pause=1.5)
    n = c.wait_node(None, rx="^Offline mode", timeout=4)
    sw = c.find("switch", rx="Offline")
    wshot(c, "privacy-offline", crop=[x for x in [n] + sw if x], pad=60, minsize=(640, 150))
    click(c, "list item", "Keyboard Shortcuts", pause=1.5)
    row = c.wait_node(None, rx="^Reading Statistics", timeout=4)
    if row:
        c.click_node(row)
        time.sleep(1.2)
        ui(c)
        wshot(c, "shortcut-rebind", crop=(440, 76, 1090, 760), pad=0)
    close(c)
    click(c, "list item", "Library", pause=1.5)
    if pbox:
        x0, y0 = pbox[0] + 6, pbox[1] + 6
        wshot(c, "prefs-banner-setting", crop=(x0 + 296, y0 + 115, x0 + 851, y0 + 245), pad=0)
    c.check("prefs details", True)


@scenario
def _wiki_map_detail(c):
    """The book map up close, and the vertical text up close."""
    from scenarios_map_keys import MAP_SCRIPT, map_label
    big = os.path.join(c.dir, "big.epub")
    import subprocess
    import sys
    subprocess.run([sys.executable, os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "make-big-epub.py"),
                    big, "900"], check=True, capture_output=True)
    c.books["big"] = big
    no_rings()
    c.start(books=("big",), settings=settings(), extra_env={"BOOKX_DEV_SCRIPT": MAP_SCRIPT})
    c.wait_node("slider", timeout=40)
    time.sleep(16)
    c.sb.key("m")
    area = c.wait(lambda: map_label(c), 8)
    time.sleep(1.0)
    if area:
        x0, y0, x1, y1 = c.screen_box(map_label(c))
        c.sb.move_to(x0 + 14 + 22 * 6.5, y0 + 14 + 15 + 15)
        time.sleep(0.8)
        wshot(c, "book-map-detail", park=False, crop=(x0, y0, x1, y1), pad=10)
    c.check("map detail", area)


@scenario
def _wiki_vertical_detail(c):
    if not c.books.get("vsynth"):
        raise Skip("vsynth not available")
    no_rings()
    c.start(books=("vsynth",), settings=settings())
    c.open_book("縦書き")
    time.sleep(2.5)
    wshot(c, "reader-vertical-detail", crop=(640, 100, 1200, 700), pad=0)
    c.check("vertical detail", True)


# ---- newer features: grouped editions, audiobooks, own vocabulary, comics ----------------------------------

@scenario
def _wiki_lib_works(c):
    """Two editions of one book grouped on one card, and the list of editions."""
    from scenarios_works import _seed
    no_rings()
    c.start(books=("alice", "tiny"), settings=settings(), seed=_seed)
    time.sleep(2.5)
    badge = c.wait_node("button", rx=r"^2 editions: show the list", timeout=10)
    cards = c.find("table cell", rx="Alice")
    if cards:
        wshot(c, "library-works-badge", crop=cards[:1], pad=30, minsize=(360, 330))
    if badge:
        before = snap(c)
        c.click_node(badge)
        c.wait_node("button", rx=r"^Open Alice.*\)$", timeout=5)
        time.sleep(1.0)
        wshot(c, "library-works-editions", crop=popup_box(before, snap(c)), pad=8)
    c.check("works shot", badge)


@scenario
def _wiki_audiobook(c):
    """The audiobook player."""
    from scenarios_audiobook import _start, _open
    _start(c, settings=settings())
    r = _open(c)
    time.sleep(1.5)
    c.sb.key("space")
    time.sleep(2.5)
    c.sb.key("space")
    time.sleep(0.8)
    wshot(c, "audiobook-player")
    c.check("player shot", r)


@scenario
def _wiki_vocab_own(c):
    """Your own vocabulary: the Add to Vocabulary dialog with a term and a definition."""
    from scenarios import select_word
    reader_start(c)
    select_word(c)
    add = c.wait_node("button", "Add to Vocabulary", timeout=6)
    if add:
        time.sleep(0.5)
        before = snap(c)
        c.click_node(add)
        time.sleep(1.5)
        c.sb.type("my own word")
        c.sb.key("Tab")
        c.sb.type("A short definition of my own.")
        time.sleep(0.8)
        save = c.find("button", "Save")
        if save:
            x1, y0 = c.screen_box(save[0])[2], c.screen_box(save[0])[1]
            wshot(c, "reader-vocab-add", crop=(x1 - 440, y0 - 16, x1 + 16, y0 + 500), pad=0)
    c.check("vocab dialog", add)


@scenario
def _wiki_reader_comic2(c):
    """The comic reader in the two-page layout, and a manga read right to left."""
    if not c.books.get("comic") or not c.books.get("manga"):
        raise Skip("no test comics")
    reader_start(c, script="wait:2500,read:1,wait:4000,reader:comic-mode=spread,wait:1500", books=("comic",))
    wshot(c, "reader-comic-spread")
    c.check("spread shot", True)


@scenario
def _wiki_reader_ruler(c):
    """The reading ruler (Alt+Down)."""
    reader_start(c)
    c.sb.key("Down", ("Alt",))
    time.sleep(2.0)
    wshot(c, "reader-ruler", crop=(256, 120, 1024, 620), pad=0)
    c.check("ruler shot", True)
