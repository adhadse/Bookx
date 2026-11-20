"""Grouped editions with real input (PP-G-5): the edition badge on a cover, the edition list, Prefer and
Unlink with real clicks and keys, the details panel, in light and dark and at 900x600 and 1280x800.
Screenshots: <results>/works_*-*.png. Imported by scenarios.py."""
import json
import os
import shutil
import subprocess
import time

from harness import scenario

DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"


def _seed(c):
    """A second edition of Alice (another publisher) and the setting that groups editions."""
    src = c.books["alice"]
    copy = os.path.join(c.dir, "alice-second.epub")
    shutil.copy(src, copy)
    for args in (["meta", "set", copy, "--publisher", "Second Edition Press"],
                 ["library", "add", "--allow-duplicates", copy]):
        r = subprocess.run([c.cli] + args, capture_output=True, text=True, env=c.sb.env)
        if r.returncode != 0:
            raise RuntimeError(f"{args}: {r.stdout}{r.stderr}")
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "config.json"), "w") as f:
        json.dump({"group_editions": True}, f)


def _origin(c):
    """AT-SPI window coordinates -> screen: a 900x600 window is centred on the 1280x800 screen."""
    w = getattr(c, "works_window", None)
    if not w:
        return (7, 37)
    ww, wh = w
    return (int((1280 - ww) / 2) + 7, int(32 + (800 - 32 - wh) / 2) + 5)


def _click(c, node):
    c.sb.click(*node.center(_origin(c)))


def _start(c, settings="", window=None):
    c.works_window = tuple(int(v) for v in window.split("x")) if window else None
    env = {"BOOKX_SIZE": window} if window else None
    c.start(books=("alice", "tiny"), settings=settings, seed=_seed, extra_env=env)
    time.sleep(2.5)


def _badge(c):
    n = c.find("button", rx=r"^2 editions: show the list")
    return n[0] if n else None


def _rows(c):
    return [n for n in c.find("button", rx=r"^Open Alice.*\)$")]


def _db(c, sql):
    import sqlite3
    con = sqlite3.connect(os.path.join(c.dir, "lib", "bookx-library.db"))
    try:
        return list(con.execute(sql))
    finally:
        con.close()


def _look(c, name, settings, window=None):
    _start(c, settings, window)
    b = c.wait_node("button", rx=r"^2 editions: show the list", timeout=10)
    if not c.check("the two Alice editions show as one card with a badge of 2", b):
        c.shot("no-badge")
        return
    cards = c.find("table cell", rx="Alice|Masque")
    c.check("the grid has one card for Alice and one for the other book", len(cards) == 2, str([n.name for n in cards]))
    c.shot("grid")
    # a real click on the badge opens the list of editions
    _click(c, b)
    first = c.wait_node("button", rx=r"^Open Alice.*\)$", timeout=5)
    c.check("the badge opens the edition list", first)
    rows = _rows(c)
    c.check("one Open button per edition", len(rows) == 2, str(len(rows)))
    c.check("every edition row has Send, Open first and Unlink with a name",
            all(c.find("button", rx=pat) for pat in (r"^Send Alice", r"^Open Alice.* first$", r"^Unlink Alice")))
    c.shot("editions")
    return b


@scenario
def works_editions_light_wide(c):
    b = _look(c, "light", "")
    if not b:
        return
    # Open first on the second edition: a real click, the catalogue keeps it
    prefer = c.find("button", rx=r"^Open Alice.* first$")
    enabled = [n for n in prefer if n.sensitive]
    c.check("the edition that opens first is marked (its button is off)", len(enabled) == 1, str(len(enabled)))
    _click(c, enabled[0])
    time.sleep(1.5)
    pref = _db(c, "select book, preferred from work_links order by book")
    c.check("choosing an edition saves the choice (the automatic group became a link)",
            len(pref) == 2 and sum(p[1] for p in pref) == 1, str(pref))
    c.check("the popover closed after the click", not _rows(c))
    # reopen: the chosen edition is the one that is off now
    _click(c, _badge(c))
    c.wait_node("button", rx=r"^Open Alice.*\)$", timeout=5)
    time.sleep(0.5)
    off = [n for n in c.find("button", rx=r"^Open Alice.* first$") if not n.sensitive]
    c.check("after reopening, the chosen edition is the one marked", len(off) == 1)
    c.shot("editions-preferred")
    # keyboard: Escape closes, the focus stays in the window
    c.sb.key("Escape")
    time.sleep(0.6)
    c.check("Escape closes the list", not _rows(c))
    # a card click selects the group: the details panel lists the editions
    cards = c.find("table cell", rx="Alice")
    _click(c, cards[0])
    c.check("selecting the card opens the details panel", c.wait_node("button", "Properties", timeout=5))
    time.sleep(1.0)
    c.check("the panel has an Editions list with a row per edition",
            len(c.find("button", rx=r"^Unlink Alice")) == 2, str(len(c.find("button", rx=r"^Unlink Alice"))))
    c.shot("details")
    # Unlink in the panel: one edition leaves, the pair dissolves, two Alice cards are shown
    _click(c, c.find("button", rx=r"^Unlink Alice")[0])
    time.sleep(1.5)
    c.check("unlinking the edition splits the card in two",
            c.wait(lambda: len(c.find("table cell", rx="Alice")) == 2, 6) and not _badge(c),
            str([n.name for n in c.find("table cell", rx="Alice")]))
    apart = _db(c, "select book from work_apart")
    c.check("the unlinked book is remembered so it is not grouped again", len(apart) == 1, str(apart))
    c.shot("split")
    c.check("the app is still alive", c.is_alive())


@scenario
def works_editions_dark_wide(c):
    b = _look(c, "dark", DARK)
    if b:
        c.sb.key("Escape")
        time.sleep(0.5)
        _click(c, c.find("table cell", rx="Alice")[0])
        time.sleep(1.2)
        c.shot("details")
    c.check("the app is still alive", c.is_alive())


@scenario
def works_editions_light_small(c):
    b = _look(c, "light-small", "", window="900x600")
    if b:
        c.sb.key("Escape")
        time.sleep(0.5)
        _click(c, c.find("table cell", rx="Alice")[0])
        time.sleep(1.2)
        c.shot("details")
    c.check("the app is still alive", c.is_alive())


@scenario
def works_editions_dark_small(c):
    b = _look(c, "dark-small", DARK, window="900x600")
    if b:
        c.sb.key("Escape")
        time.sleep(0.5)
    c.check("the app is still alive", c.is_alive())


@scenario
def works_editions_keyboard(c):
    """Everything on the list is reachable with the keyboard: Tab moves between the buttons of a row,
    Enter presses one."""
    _start(c)
    b = c.wait_node("button", rx=r"^2 editions: show the list", timeout=10)
    if not c.check("the badge is there", b):
        return
    _click(c, b)
    c.wait_node("button", rx=r"^Open Alice.*\)$", timeout=5)
    time.sleep(0.5)
    focus0 = [n for n in c.find() if n.focused]
    c.check("the list takes the keyboard focus", bool(focus0), "nothing focused")
    # Tab until the "Open first" button of an enabled row is focused, then Enter
    hit = False
    for _ in range(14):
        foc = [n for n in c.find("button") if n.focused]
        if foc and "first" in foc[0].name and "Open Alice" in foc[0].name:
            hit = True
            break
        c.sb.key("Tab")
        time.sleep(0.25)
    c.check("Tab reaches the Open-first button of an edition", hit)
    if hit:
        c.sb.key("Return")
        time.sleep(1.5)
        pref = _db(c, "select book, preferred from work_links order by book")
        c.check("Enter on it chooses that edition", len(pref) == 2 and sum(p[1] for p in pref) == 1, str(pref))
    c.shot("keyboard")
    c.check("the app is still alive", c.is_alive())
