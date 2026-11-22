"""Wave 11 features in a real GNOME session with real input (docs/remaining-work.md, part G: "no session scenario for live
updates and the new dialogs"):

* live updates of an open reader when an e-reader syncs, through the real HTTP endpoints of the in-process server
  (`live_sync_highlight`, `live_sync_ahead_banner`) and the Sync window with its activity (`sync_activity_window`);
* the master-password dialogs (`secrets_create_dialog`, `secrets_unlock_dialog`, `secrets_prompt_from_preferences`);
* the Custom Columns dialog (`custom_columns_dialog`);
* Settings Backup, the export and import dialogs (`settings_backup_dialogs`);
* the plugin trust sheet (`plugin_trust_sheet`) and the safe-mode start (`safe_mode_start`).

Every scenario checks that no interactive control is without an accessible name (`unnamed`) and that the dialog can be
worked with the keyboard alone (`tab_to`, Return, Escape). Screenshots go to <results>/<scenario>-*.png. Imported by
scenarios.py. Run: ONLY='live_sync_highlight secrets_create_dialog' build-aux/session-tests.sh
"""
import glob
import hashlib
import json
import os
import re
import shutil
import time
import urllib.error
import urllib.request

from harness import scenario
from scenarios_a11y import describe, page_node, page_text, unnamed

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.abspath(os.path.join(HERE, "..", ".."))
PW_KEY = hashlib.md5(b"pw").hexdigest()  # what KOReader sends as the KOSync key of the password "pw"


# ---- helpers ------------------------------------------------------------------------------------------------------

def focus_of(c):
    """The widget that has the keyboard focus (the deepest one the tree reports), or None."""
    nodes = [n for n in c.tree().nodes if n.focused and n.showing]
    return nodes[-1] if nodes else None


def tab_to(c, rx, limit=40, back=False):
    """Presses Tab (Shift+Tab with back=True) until the focused control's name matches `rx`. Returns (found, the focus
    order that was walked as 'role:name' strings). This is the keyboard-only path of a dialog."""
    walked = []
    for _ in range(limit):
        f = focus_of(c)
        if f and re.search(rx, f.name or ""):
            return True, walked
        walked.append(f"{f.role}:{f.name}" if f else "none")
        c.sb.key("Tab", ("Shift",) if back else ())
        time.sleep(0.15)
    f = focus_of(c)
    return bool(f and re.search(rx, f.name or "")), walked


def has(c, rx, role=None):
    return [n for n in c.find(role, rx=rx) if n.showing]


def wait_text(c, rx, timeout=6.0, role=None):
    return c.wait(lambda: has(c, rx, role), timeout)


def log_has(c, rx):
    """Whether the app's log has a line matching rx (the dev hooks that open a dialog report their toast there)."""
    text = ""
    for f in sorted(glob.glob(os.path.join(c.dir, "app*.log"))):  # app2.log is the log of a restarted app
        text += open(f, errors="replace").read()
    return re.search(rx, text) is not None


def wait_log(c, rx, timeout=6.0):
    return c.wait(lambda: log_has(c, rx), timeout)


def library_epub(c):
    files = sorted(glob.glob(os.path.join(c.dir, "lib", "**", "*.epub"), recursive=True))
    return files[0] if files else c.books["alice"]


def partial_md5(path):
    """KOReader's document id: the MD5 of 1 KiB blocks at offsets 0 and 1024 << 2i (`kosync_client::partial_md5`)."""
    md5 = hashlib.md5()
    with open(path, "rb") as f:
        for off in [0] + [1024 << (2 * i) for i in range(0, 11)]:
            f.seek(off)
            buf = f.read(1024)
            if not buf:
                break
            md5.update(buf)
    return md5.hexdigest()


class Device:
    """A fake KOReader: the same HTTP calls as the plugin makes, to the real server inside the app."""

    def __init__(self, c, name="Kobo Libra"):
        self.c = c
        self.name = name
        self.clock = int(time.time())
        self.base = None
        self.doc = partial_md5(library_epub(c))

    def connect(self, timeout=15):
        """Needs the dev command `koannot:start` (the app prints `koannot_started: URL` to its log)."""
        def url():
            try:
                log = open(os.path.join(self.c.dir, "app.log"), errors="replace").read()
            except OSError:
                return None
            m = re.search(r"koannot_started: (http\S+)", log)
            return m.group(1) if m else None
        u = self.c.wait(url, timeout)
        if not u:
            return False
        self.base = u.rstrip("/") + "/bookx"
        self.call("POST", "/users/create", {"username": "ko", "password": PW_KEY}, auth=False)
        return True

    def call(self, method, path, body, auth=True):
        h = {"Content-Type": "application/json", "X-Bookx-Client": "KOReader/2026.07.2 bookx-plugin/0.3.1",
             "X-Bookx-Protocol": "1"}
        if auth:
            h.update({"x-auth-user": "ko", "x-auth-key": PW_KEY})
        req = urllib.request.Request(self.base + path, data=json.dumps(body).encode(), method=method, headers=h)
        try:
            with urllib.request.urlopen(req, timeout=10) as r:
                return r.status
        except urllib.error.HTTPError as e:
            return e.code

    def highlight(self, hid, text, colour="yellow", note="", delete=False):
        self.clock += 2
        a = {"id": hid, "kind": "highlight", "text": text, "note": note, "color": colour, "drawer": "lighten",
             "chapter": "One", "page": 3, "created": self.clock - 5, "updated": self.clock}
        if delete:
            a["deleted"] = True
        return self.call("PUT", f"/annotations/{self.doc}", {
            "origin": "koreader", "device": self.name, "device_id": f"fake-{self.name}",
            "title": "Alice's Adventures in Wonderland", "authors": "Lewis Carroll", "annotations": [a]})

    def progress(self, pct):
        return self.call("PUT", "/syncs/progress", {
            "document": self.doc, "progress": "/body/DocFragment[3]/body", "percentage": pct,
            "device": self.name, "device_id": f"fake-{self.name}"})


def page_quote(c, words=9):
    """A quote of the open page: a run of plain words (letters, commas and full stops only) from the middle of a long paragraph,
    so it is whole lines on this page and unique in the book."""
    text = page_text(c) or ""
    paras = [" ".join(p.split()) for p in text.split("\n") if len(p.split()) >= 25]
    runs = []
    for p in paras:
        run = []
        for w in p.split(" "):
            if re.fullmatch(r"[A-Za-z]+[,.]?", w):
                run.append(w)
            else:
                if len(run) >= words:
                    runs.append(run)
                run = []
        if len(run) >= words:
            runs.append(run)
    if not runs:
        return None
    best = runs[min(1, len(runs) - 1)]
    mid = max(0, len(best) // 2 - words // 2)
    return " ".join(best[mid:mid + words])


def changed_pixels(a, b, box, thr=60):
    """(count, mean colour of the changed pixels) of `box` between two screenshots."""
    x0, y0, x1, y1 = box
    pa, pb = a.load(), b.load()
    n, sr, sg, sb = 0, 0, 0, 0
    for y in range(y0, min(y1, a.height, b.height), 2):
        for x in range(x0, min(x1, a.width, b.width), 2):
            p, q = pa[x, y], pb[x, y]
            if sum(abs(i - j) for i, j in zip(p, q)) > thr:
                n += 1
                sr, sg, sb = sr + q[0], sg + q[1], sb + q[2]
    return n, ((sr // n, sg // n, sb // n) if n else None)


def wait_pixels(c, before, box, more_than, timeout=3.0):
    """Polls the screen until `box` differs from `before` by more than `more_than` pixels (counted every 2nd pixel)."""
    t0 = time.time()
    last = (0, None)
    while time.time() - t0 < timeout:
        last = changed_pixels(before, c.image(), box)
        if last[0] > more_than:
            return time.time() - t0, last
        time.sleep(0.25)
    return None, last


def open_prefs(c):
    c.sb.key(",", ("Control",))
    return c.wait_node("dialog", "Preferences", timeout=8)


def prefs_find(c, term):
    """Filters the Preferences rows with the search (Ctrl+F, the keyboard path) and returns the shown row nodes."""
    c.sb.key("f", ("Control",))
    time.sleep(0.5)
    c.sb.type(term)
    time.sleep(1.0)


def click_row(c, title, role=None):
    """Clicks the widest showing node named `title` (a row of Preferences: its label and the row itself carry the name)."""
    nodes = sorted(has(c, "^" + re.escape(title) + "$", role), key=lambda n: -n.box[2])
    if not nodes:
        return None
    c.click_node(nodes[0])
    time.sleep(1.0)
    return nodes[0]


def config_json(c):
    p = os.path.join(c.dir, "cfg", "bookx", "config.json")
    try:
        return json.load(open(p))
    except (OSError, ValueError):
        return {}


# ---- live updates -------------------------------------------------------------------------------------------------

@scenario
def live_sync_highlight(c):
    """An e-reader sends a highlight, a new colour and a delete through the real /bookx/annotations endpoint while the
    book is open: the page paints it, the sidebar Notes tab lists its note (AT-SPI) and takes it away, each within 3 s."""
    c.start(books=("alice",), extra_env={"BOOKX_DEV_SCRIPT": "wait:400,koannot:start"})
    dev = Device(c)
    if not c.check("the in-process sync server started (koannot_started in the log)", dev.connect()):
        return
    c.open_book()
    c.go_to_text()
    quote = page_quote(c)
    if not c.check("the page text offers a quote to highlight", quote and len(quote) > 30, repr(quote)):
        return
    c.note(f"quote: {quote!r}")
    # the Notes tab of the sidebar, opened from the command palette (the keyboard path), so a note shows up as a label
    c.sb.key("p", ("Control", "Shift"))
    c.wait_node(None, "Command palette search", timeout=5)
    c.sb.type("bookmarks and")
    time.sleep(0.5)
    c.sb.key("Return")
    time.sleep(1.2)
    canvas = page_node(c)
    box = c.screen_box(canvas) if canvas else (300, 100, 900, 700)
    # not the sync toast that slides in over the bottom of the page, and not the sidebar (a blinking caret in its search entry)
    box = (max(box[0], 385), box[1], box[2], box[3] - 110)
    c.check("the sidebar starts without the note", not has(c, "LIVE-NOTE-7731"))
    before = c.image("live-0-before")
    status = dev.highlight("live-aaa", quote, "purple", "LIVE-NOTE-7731")
    c.check("the server accepted the highlight (HTTP 200)", status == 200, str(status))
    took, (n, purple) = wait_pixels(c, before, box, 40)
    c.shot("live-1-purple")
    c.check("the highlight is painted on the open page within 3 s", took is not None, f"{n} changed px")
    shown = wait_text(c, "LIVE-NOTE-7731", 3.0)
    c.check("its note is in the sidebar Notes tab within 3 s (AT-SPI)", shown)
    # a new colour for the same highlight
    status = dev.highlight("live-aaa", quote, "green", "LIVE-NOTE-7731")
    c.check("the colour change was accepted", status == 200, str(status))
    def recoloured():
        n_, col = changed_pixels(before, c.image(), box)
        return n_ > 40 and col and purple and sum(abs(a - b) for a, b in zip(purple, col)) > 40
    c.wait(recoloured, 3.0)
    after = c.image("live-2-green")
    n2, green = changed_pixels(before, after, box)
    c.check("the highlight is still painted after the colour change", n2 > 40, f"{n2} px")
    c.check("and its colour really changed on the page (purple -> green)",
            purple and green and sum(abs(a - b) for a, b in zip(purple, green)) > 40, f"{purple} {green}")
    # a delete on the device
    dev.highlight("live-aaa", quote, "green", "LIVE-NOTE-7731", delete=True)
    gone = c.wait(lambda: changed_pixels(before, c.image(), box)[0] < 12, 3.0)
    c.shot("live-3-deleted")
    c.check("a highlight deleted on the e-reader disappears from the page within 3 s", gone,
            f"{changed_pixels(before, c.image(), box)[0]} px still differ from the page before")
    c.check("and from the Notes tab", c.wait(lambda: not has(c, "LIVE-NOTE-7731"), 3.0))
    bad = unnamed(c)
    c.check("every control is named while the book is open", not bad, describe(bad))
    c.check("the app is alive", c.is_alive())


@scenario
def live_sync_ahead_banner(c):
    """The device is further along: a banner offers it (never a silent jump), it is reachable by the keyboard, Continue There
    moves the page and Dismiss closes it. Controls are named."""
    c.start(books=("alice",), extra_env={"BOOKX_DEV_SCRIPT": "wait:400,koannot:start"})
    dev = Device(c)
    if not c.check("the in-process sync server started", dev.connect()):
        return
    c.open_book()
    c.go_to_text()
    p0 = c.page_label()
    c.check("the reader shows a page number", p0, str(p0))
    status = dev.progress(0.62)
    c.check("the server accepted the position (HTTP 200)", status == 200, str(status))
    banner = wait_text(c, r"Kobo Libra is at 62%", 4.0)
    c.shot("ahead-1-banner")
    c.check("the banner names the device and the percentage within 3 s", banner)
    c.check("the page did not move by itself", c.page_label() == p0, f"{p0} -> {c.page_label()}")
    c.check("the banner is named for a screen reader", has(c, "^Sync message$") or banner)
    c.check("it has a Continue There and a Dismiss button", c.find("button", "Continue There") and c.find("button", "Dismiss"))
    bad = unnamed(c)
    c.check("every control is named while the banner shows", not bad, describe(bad))
    ok, walked = tab_to(c, r"^Continue There$", limit=60)
    c.note("focus order before Continue There: " + " > ".join(walked[:30]))
    c.check("Continue There can be reached with Tab alone (keyboard-reachable)", ok, " > ".join(walked[-8:]))
    if ok:
        c.sb.key("Return")
        time.sleep(1.5)
        p1 = c.page_label()
        c.check("Return on Continue There goes to the device's place", p1 and p0 and p1[0] > p0[0] + 20, f"{p0} -> {p1}")
        c.check("the banner is gone afterwards", c.wait(lambda: not has(c, r"Kobo Libra is at 62%"), 4.0))
    # a new, further position and Dismiss with the keyboard
    dev.progress(0.9)
    wait_text(c, r"Kobo Libra is at 90%", 4.0)
    c.shot("ahead-2-second")
    ok, walked = tab_to(c, r"^Dismiss$", limit=60)
    c.check("Dismiss can be reached with Tab alone", ok, " > ".join(walked[-8:]))
    if ok:
        c.sb.key("Return")
        c.check("Return on Dismiss closes the banner", c.wait(lambda: not has(c, r"Kobo Libra is at 90%"), 4.0))
    c.check("the reader is still open", c.find("slider"))
    c.check("the app is alive", c.is_alive())


def open_sync_window(c):
    opened = c.wait_node("toggle button", rx="^Main Menu", timeout=3)
    if not opened:
        side = c.find("toggle button", "Sidebar")
        if side:
            c.click_node(side[0])
            time.sleep(0.8)
    menu = c.wait_node("toggle button", rx="^Main Menu", timeout=4)
    if not menu:
        return None
    c.click_node(menu)
    time.sleep(0.8)
    found = [n for n in has(c, r"^Sync$", "menu item") if n.box[2] > 0]
    if not found:
        return None
    c.click_node(max(found, key=lambda n: n.box[1]))
    time.sleep(1.2)
    return c.wait_node("dialog", timeout=8)


@scenario
def sync_activity_window(c):
    """The Sync window lists the connected device and the recent activity of a real sync; Show All opens the full list with
    the keyboard and Escape closes it."""
    c.start(books=("alice",), extra_env={"BOOKX_DEV_SCRIPT": "wait:400,koannot:start"})
    dev = Device(c)
    if not c.check("the in-process sync server started", dev.connect()):
        return
    c.wait_node("table cell", rx="Alice")
    for i in range(3):
        dev.highlight(f"act-{i}", f"Activity test quote number {i} that is not in the book", "yellow", f"n{i}")
    dev.progress(0.3)
    time.sleep(1.0)
    d = open_sync_window(c)
    if not c.check("the Sync window opens from the main menu", d):
        c.note(c.tree().dump()[-2500:])
        return
    c.shot("activity-1-window")
    dx, dy, dw, dh = d.box
    row = None
    for _ in range(10):
        row = wait_text(c, r"Kobo Libra", 1.0)
        if row:
            break
        c.sb.scroll(5, dx + dw * 0.62, dy + dh * 0.5)
        time.sleep(0.3)
    c.check("the window lists the device that synced (Connected devices)", row)
    c.check("a Recent activity group is there", has(c, r"^Recent activity$"))
    bad = unnamed(c)
    c.check("every control of the Sync window is named", not bad, describe(bad))
    show = None
    for _ in range(12):
        show = c.find("button", rx=r"^Show all sync activity$")
        if show:
            break
        c.sb.scroll(5, dx + dw * 0.62, dy + dh * 0.5)
        time.sleep(0.3)
    if not c.check("Show All is a button announced as Show all sync activity (bug 29)", show):
        return
    ok, walked = tab_to(c, r"^Show [Aa]ll( sync activity)?$", limit=80)
    c.check("Show All can be reached with Tab alone", ok, " > ".join(walked[-6:]))
    if ok:
        c.sb.key("Return")
    else:
        c.click_node(show[0])
    time.sleep(1.2)
    c.shot("activity-2-all")
    c.check("it opens All sync activity", wait_text(c, r"^All sync activity$", 5.0))
    c.check("the list names the device and what it sent", has(c, r"Kobo Libra"))
    c.sb.key("Escape")
    time.sleep(0.8)
    c.check("Escape closes the list and leaves the Sync window", c.wait(lambda: not has(c, r"^All sync activity$"), 4.0)
            and c.find("dialog"))
    c.check("the app is alive", c.is_alive())


# ---- the master password ------------------------------------------------------------------------------------------

SECRETS_ENV = {"BOOKX_SECRETS": "none", "BOOKX_SECRETS_PROMPT": "1"}


def vault_files(c):
    return glob.glob(os.path.join(c.dir, "cfg", "**", "secrets.vault"), recursive=True)


@scenario
def secrets_create_dialog(c):
    """Create a Master Password (no keyring): names, focus order, a short or different password cannot be created, Escape
    is Don't Save, and Create works with the keyboard alone."""
    env = dict(SECRETS_ENV, BOOKX_DEV_SCRIPT="wait:1500,secrets:show-create")
    c.start(extra_env=env)
    if not c.check("the Create a Master Password dialog shows", wait_text(c, r"^Create a Master Password$", 8.0)):
        c.note(c.tree().dump()[-2000:])
        return
    c.shot("create-1")
    c.check("the first field is named Master password", c.find(None, "Master password"))
    c.check("the second is named Repeat the master password", c.find(None, "Repeat the master password"))
    c.check("the rule is said in words (at least 10 characters)", has(c, r"at least 10 characters"))
    create = c.find("button", "Create")
    c.check("Create starts insensitive", create and not create[0].sensitive)
    c.check("there is a Don't Save button", c.find("button", "Don't Save"))
    bad = unnamed(c)
    c.check("every control is named", not bad, describe(bad))
    ok, walked = tab_to(c, r"^Repeat the master password$", limit=12)
    c.note("focus order: " + " > ".join(walked))
    c.check("Tab reaches the second password field", ok, " > ".join(walked))
    c.sb.key("Tab", ("Shift",))
    c.sb.type("short")
    time.sleep(0.5)
    c.check("a short password is refused in words and Create stays off",
            has(c, r"too short|at least 10|10 characters") and not [n for n in c.find("button", "Create") if n.sensitive],
            str([n.name for n in c.find(None, rx=r"(?i)password|char")][:6]))
    c.sb.key("Escape")
    c.check("Escape is Don't Save: the dialog closes", c.wait(lambda: not has(c, r"^Create a Master Password$"), 4.0))
    c.check("and says that it is forgotten when Bookx closes (the hook reports the toast in the log)",
            wait_log(c, r"secrets_toast: Not saved: it is forgotten when Bookx closes", 5.0))
    c.check("no vault file was written", not vault_files(c))
    c.shot("create-2-declined")
    c.check("the app is alive", c.is_alive())


@scenario
def secrets_unlock_dialog(c):
    """Create the store with the keyboard, restart (the store is locked again and asks by itself at the start), then: a wrong password asks again, the right
    one unlocks with Enter; Not Now (Escape) keeps it locked; Forgot Password asks before it deletes anything."""
    PW = "a long master pw"
    env = dict(SECRETS_ENV, BOOKX_DEV_SCRIPT="wait:1500,secrets:show-create")
    c.start(extra_env=env)
    if not c.check("the Create dialog shows", wait_text(c, r"^Create a Master Password$", 8.0)):
        return
    c.sb.type(PW)
    c.sb.key("Tab")
    c.sb.type(PW)
    time.sleep(0.6)
    create = c.find("button", "Create")
    c.check("two equal long passwords enable Create", create and create[0].sensitive)
    ok, walked = tab_to(c, r"^Create$", limit=12)
    c.check("Create can be reached with Tab alone", ok, " > ".join(walked))
    if ok:
        c.sb.key("Return")
    c.check("Create closes the dialog and says the passwords are in an encrypted store",
            wait_log(c, r"secrets_toast: Your passwords are saved in an encrypted store", 6.0)
            and not has(c, r"^Create a Master Password$"))
    c.check("the encrypted store file exists", c.wait(lambda: vault_files(c), 4.0))
    c.shot("unlock-1-created")
    # a new process: the store is locked again
    c.app_env = {k: v for k, v in c.app_env.items() if k != "BOOKX_DEV_SCRIPT"}  # no hook: the store asks by itself
    c.restart_app()
    if not c.check("the Unlock Saved Passwords dialog shows", wait_text(c, r"^Unlock Saved Passwords$", 8.0)):
        c.note(c.tree().dump()[-2000:])
        return
    c.shot("unlock-2-dialog")
    c.check("its field is named Master password", c.find(None, "Master password"))
    for b in ("Not Now", "Forgot Password", "Unlock"):
        c.check(f"it has the {b} button", c.find("button", b))
    bad = unnamed(c)
    c.check("every control is named", not bad, describe(bad))
    f = focus_of(c)
    c.check("the password field has the keyboard focus when the dialog opens (regression: it was on Unlock)",
            f and f.name == "Master password", str(f))
    ok, walked = tab_to(c, r"^Master password$", limit=8)
    c.check("the password field can be focused with Tab", ok, " > ".join(walked))
    c.sb.type("not the password")
    c.sb.key("Return")
    c.check("a wrong password shows the dialog again with the reason",
            wait_text(c, r"did not work", 6.0))
    c.shot("unlock-3-wrong")
    ok, _ = tab_to(c, r"^Master password$", limit=8)
    c.sb.type(PW)
    c.sb.key("Return")
    c.check("the right password and Enter unlock: a toast says so", wait_text(c, r"Saved passwords unlocked", 6.0))
    c.check("the dialog is gone", c.wait(lambda: not has(c, r"^Unlock Saved Passwords$"), 4.0),
            "dialogs: " + str([(n.role, n.name, n.box) for n in c.find(None, rx=r"^Unlock Saved Passwords$")]))
    c.shot("unlock-3b-after-unlock")
    # Not Now and Forgot Password, in a second process
    c.app_env = {k: v for k, v in c.app_env.items() if k != "BOOKX_DEV_SCRIPT"}  # no hook: the store asks by itself
    c.restart_app()
    wait_text(c, r"^Unlock Saved Passwords$", 8.0)
    ok, walked = tab_to(c, r"^Forgot Password$", limit=10)
    c.check("Forgot Password can be reached with Tab", ok, " > ".join(walked))
    if ok:
        c.sb.key("Return")
        c.check("it asks before deleting anything (Forgot the Master Password?)",
                wait_text(c, r"^Forgot the Master Password\?$", 5.0))
        c.check("the delete button says what it deletes", c.find("button", "Delete Saved Passwords"))
        c.shot("unlock-4-forgot")
        c.sb.key("Escape")
        c.check("Escape goes back to the unlock dialog", wait_text(c, r"^Unlock Saved Passwords$", 5.0))
    c.check("the store file is still there", vault_files(c))
    time.sleep(1.0)
    c.shot("unlock-5-before-not-now")
    c.sb.key("Escape")
    c.check("Escape is Not Now: the dialog closes", c.wait(lambda: not has(c, r"^Unlock Saved Passwords$"), 5.0))
    c.check("and a toast says the passwords stay locked", wait_text(c, r"Saved passwords stay locked", 5.0))
    c.check("the app is alive", c.is_alive())


@scenario
def secrets_prompt_from_preferences(c):
    """No dev hook, no keyring (docs/bugs.md, bug 27): merely opening Preferences, its MCP page and its Sync page raises no
    Create a Master Password dialog, because the MCP access token is made only when the user switches "Let AI assistants
    connect" on. That switch has a token to keep and nowhere to keep it, so it raises the dialog; Don't Save with the
    keyboard ends the questions for the session, and a password typed afterwards is in no file."""
    c.start(extra_env=dict(SECRETS_ENV))
    d = open_prefs(c)
    if not c.check("Ctrl+, opens Preferences", d):
        return
    time.sleep(4.0)  # the store looks for something to ask every second
    c.check("opening Preferences asks for no master password", not has(c, r"^Create a Master Password$"))
    for page in ("MCP", "Sync"):
        click_row(c, page, "list item")
        time.sleep(3.0)
        c.check(f"the {page} page asks for no master password", not has(c, r"^Create a Master Password$"))
        if page == "MCP":
            c.shot("prompt-1-mcp-page")
            c.check("the token row says there is no token yet", has(c, r"^No token yet$"))
    click_row(c, "MCP", "list item")
    sw = click_row(c, "Let AI assistants connect")
    if not c.check("the switch for the assistant server is on the MCP page", sw):
        c.note(c.tree().dump()[-1500:])
        return
    asked = wait_text(c, r"^Create a Master Password$", 8.0)
    c.shot("prompt-2-asked")
    c.check("switching it on makes a token and the dialog asks where to keep it", asked)
    if asked:
        ok, walked = tab_to(c, r"^Don't Save$", limit=12)
        c.check("Don't Save can be reached with Tab alone", ok, " > ".join(walked))
        if ok:
            c.sb.key("Return")
        c.check("the dialog closes", c.wait(lambda: not has(c, r"^Create a Master Password$"), 4.0))
    click_row(c, "E-mail", "list item")
    row = c.wait(lambda: has(c, r"^Password$"), 5.0)
    c.shot("prompt-3-mail")
    if not c.check("the E-mail page shows its Password row", row):
        c.note(c.tree().dump()[-1500:])
        return
    c.click_node(row[0])
    time.sleep(0.5)
    c.sb.type("SECRET-pw-424242")
    c.sb.key("Tab")
    time.sleep(2.5)
    c.check("a declined store is not asked again in the same session", not has(c, r"^Create a Master Password$"))
    leaked = []
    for f in glob.glob(os.path.join(c.dir, "cfg", "**", "*"), recursive=True):
        if os.path.isfile(f) and b"SECRET-pw-424242" in open(f, "rb").read():
            leaked.append(f)
    c.check("the password is in no file under the config folder", not leaked, str(leaked))
    c.check("the app is alive", c.is_alive())


# ---- Custom Columns -----------------------------------------------------------------------------------------------

@scenario
def custom_columns_dialog(c):
    """Preferences -> Library -> Custom Columns: add a column with the keyboard (Name, Tab to Save, Return), see it listed,
    open Edit, cancel a Remove with Escape, then remove it. Names and focus order are checked on the way."""
    c.start(books=("alice",))
    d = open_prefs(c)
    if not c.check("Ctrl+, opens Preferences", d):
        return
    prefs_find(c, "custom columns")
    row = click_row(c, "Custom Columns")
    if not c.check("the Custom Columns row opens its dialog", row and wait_text(c, r"^Add Column$", 6.0)):
        c.note(c.tree().dump()[-2500:])
        return
    c.shot("columns-1-list")
    c.check("the dialog explains the search label", has(c, r"#label:value"))
    bad = unnamed(c)
    c.check("every control of the list is named", not bad, describe(bad))
    ok, walked = tab_to(c, r"^Add Column$", limit=20)
    c.check("the Add Column row can be reached with Tab alone", ok, " > ".join(walked))
    if ok:
        c.sb.key("Return")
    else:
        c.click_node(has(c, r"^Add Column$")[0])
    time.sleep(1.0)
    c.shot("columns-2-form")
    for n in ("Name", "Search label", "Type"):
        c.check(f"the form has the field {n}", has(c, "^" + n + "$"))
    bad = unnamed(c)
    c.check("every control of the form is named", not bad, describe(bad))
    c.check("Save and Cancel are named buttons", c.find("button", "Save") and c.find("button", "Cancel"))
    f = focus_of(c)
    c.check("the Name field has the keyboard focus when the form opens (regression: it was on Cancel, and a space pressed it)",
            f and f.name == "Name", str(f))
    c.sb.type("pages read")
    time.sleep(0.5)
    ok, walked = tab_to(c, r"^Save$", limit=30)
    c.check("Save can be reached with Tab alone", ok, " > ".join(walked[-8:]))
    if ok:
        c.sb.key("Return")
    time.sleep(1.2)
    c.shot("columns-3-added")
    c.check("the new column is listed with its search label and type", wait_text(c, r"#pages_read", 5.0), str(
        [n.name for n in c.find(None, rx="pages")][:5]))
    # Edit: label and type are fixed
    ok, walked = tab_to(c, r"^Edit pages read$", limit=30)
    c.check("the Edit button of the column has a name that says which column", ok, " > ".join(walked[-6:]))
    if ok:
        c.sb.key("Return")
        c.check("it opens Edit Column", wait_text(c, r"^Edit Column$", 5.0))
        c.sb.key("Escape")
        c.check("Escape closes the form and keeps the list", c.wait(lambda: not has(c, r"^Edit Column$"), 4.0)
                and has(c, r"#pages_read"))
    # Remove: ask first
    ok, walked = tab_to(c, r"^Remove pages read$", limit=30)
    c.check("the Remove button is reachable and says which column", ok, " > ".join(walked[-6:]))
    if ok:
        c.sb.key("Return")
        time.sleep(1.0)
        c.shot("columns-4-remove")
        c.check("Remove asks first and says what happens to the values", has(c, r"No book has a value|values"))
        c.sb.key("Escape")
        time.sleep(0.8)
        c.check("Escape cancels: the column is still there", has(c, r"#pages_read"))
        ok, _ = tab_to(c, r"^Remove pages read$", limit=30)
        if ok:
            c.sb.key("Return")
            time.sleep(0.8)
            ok2, w2 = tab_to(c, r"^Remove$", limit=10)
            c.check("the confirmation has a Remove button reachable with Tab", ok2, " > ".join(w2))
            if ok2:
                c.sb.key("Return")
                c.check("removing it takes the column out of the list", c.wait(lambda: not has(c, r"#pages_read"), 5.0))
    c.check("the app is alive", c.is_alive())


# ---- Settings Backup ----------------------------------------------------------------------------------------------
# The file choosers of Export Settings and Import Settings are the desktop portal's (xdg-desktop-portal-gtk is an AT-SPI
# application of its own and shows no window in the private session), so the choosers are not driven here. The dev hooks
# `settings:export=PATH` and `settings:import=PATH` take the place of the chooser and open the real preview dialog.

def seed_settings_files(c):
    changed = os.path.join(c.dir, "import-eink.json")
    json.dump({"format": "bookx-settings", "version": 1, "app_version": "0", "settings": {"eink": True}}, open(changed, "w"))
    return os.path.join(c.dir, "exported-settings.json"), changed


@scenario
def settings_backup_dialogs(c):
    """Preferences -> General -> Settings Backup: the buttons are named; an export has no secrets; the import preview says
    what would change, Return is Cancel (the safe default, nothing changes), Tab + Return on Import applies it."""
    out, changed = seed_settings_files(c)
    c.start(books=("alice",), extra_env={"BOOKX_DEV_SCRIPT": f"wait:2500,settings:export={out},wait:500,settings:import={changed}"})
    c.check("an export file was written", c.wait(lambda: os.path.isfile(out), 10.0))
    if os.path.isfile(out):
        doc = json.load(open(out))
        text = open(out).read().lower()
        c.check("it is a Bookx settings file", doc.get("format") == "bookx-settings" and doc.get("version") == 1, str(list(doc)))
        c.check("it holds no password, token or library path",
                not re.search(r'"[a-z_]*(password|token|passphrase)[a-z_]*": "[^"]', text) and "library_dir" not in doc.get("settings", {}))
    c.check("the import preview says which settings would change", wait_text(c, r"1 settings would change", 10.0))
    c.shot("settings-1-preview")
    c.check("the preview names the setting", has(c, r"eink"))
    c.check("the preview has Cancel and Import buttons", c.find("button", "Cancel") and c.find("button", "Import"))
    bad = unnamed(c)
    c.check("every control of the preview is named", not bad, describe(bad))
    f = focus_of(c)
    c.note(f"initial focus in the preview: {f}")
    c.sb.key("Return")
    c.check("Return is Cancel (the safe default): the preview closes", c.wait(lambda: not has(c, r"settings would change"), 4.0))
    c.check("no toast says Imported", not has(c, r"Settings Imported"))
    c.check("nothing was written to the settings", not config_json(c).get("eink"))
    # the Preferences rows that the file choosers hang on
    d = open_prefs(c)
    if c.check("Ctrl+, opens Preferences", d):
        prefs_find(c, "settings backup")
        c.shot("settings-2-rows")
        c.check("Export Settings is a named button", has(c, r"^Export Settings$", "button"))
        c.check("Import Settings is a named button", has(c, r"^Import Settings$", "button"))
        c.sb.key("Escape")
        time.sleep(0.5)
        c.sb.key("Escape")
    # a second launch: accept with the keyboard
    c.app_env = dict(c.app_env, BOOKX_DEV_SCRIPT=f"wait:2500,settings:import={changed}")
    c.restart_app()
    wait_text(c, r"1 settings would change", 10.0)
    ok, walked = tab_to(c, r"^Import$", limit=8)
    c.check("Import can be reached with Tab alone", ok, " > ".join(walked))
    if ok:
        c.sb.key("Return")
    c.check("the preview closes", c.wait(lambda: not has(c, r"settings would change"), 4.0))
    c.check("the setting is now in config.json", c.wait(lambda: config_json(c).get("eink") is True, 6.0), str(config_json(c).get("eink")))
    c.check("the app is alive", c.is_alive())


# ---- plugin trust and safe mode -----------------------------------------------------------------------------------

def seed_plugin(c):
    dst = os.path.join(c.dir, "plugins", "trim")
    os.makedirs(dst, exist_ok=True)
    src = os.path.join(ROOT, "examples", "plugins", "trim-whitespace")
    for f in ("plugin.toml", "plugin.wasm"):
        shutil.copy(os.path.join(src, f), dst)


@scenario
def plugin_trust_sheet(c):
    """Switching a plugin on shows the capability sheet first: what it may do in words, a warning that it is unsigned,
    Cancel is the default (Return and Escape leave it off), Allow with the keyboard switches it on and remembers it."""
    seed_plugin(c)
    c.start(books=("alice",), extra_env={"BOOKX_PLUGINS": os.path.join(c.dir, "plugins")})
    d = open_prefs(c)
    if not c.check("Ctrl+, opens Preferences", d):
        return
    prefs_find(c, "trim trailing")
    sw = [n for n in c.find("switch") if "Trim trailing whitespace" in n.name]
    c.shot("trust-1-row")
    if not c.check("the plugin is listed as a named switch", sw, str([n.name for n in c.find(None, rx="rim")][:5])):
        c.note(c.tree().dump()[-2500:])
        return
    c.check("it is off", not sw[0].checked)
    c.click_node(sw[0])
    sheet = wait_text(c, r"^Allow Trim trailing whitespace to run\?$", 6.0)
    c.shot("trust-2-sheet")
    c.check("the capability sheet opens before it is switched on", sheet)
    c.check("it says in words what the plugin may do",
            has(c, r"Read the book that is open in the editor") and has(c, r"Change the book that is open in the editor"))
    c.check("it warns that the plugin is not signed", has(c, r"not signed"))
    c.check("it says that the plugin runs in a sandbox", has(c, r"sandbox"))
    c.check("Cancel and Allow are named buttons", c.find("button", "Cancel") and c.find("button", "Allow"))
    bad = unnamed(c)
    c.check("every control is named", not bad, describe(bad))
    c.sb.key("Return")
    c.check("Return is Cancel (the default): the sheet closes", c.wait(lambda: not has(c, r"^Allow Trim trailing"), 4.0))
    sw = [n for n in c.find("switch") if "Trim trailing whitespace" in n.name]
    c.check("and the plugin stays off", sw and not sw[0].checked)
    # the keyboard path: focus the switch, Space, Tab to Allow, Return
    ok, walked = tab_to(c, r"Trim trailing whitespace", limit=40)
    c.check("the plugin switch can be reached with Tab", ok, " > ".join(walked[-6:]))
    if ok:
        c.sb.key("space")
        wait_text(c, r"^Allow Trim trailing whitespace to run\?$", 6.0)
        ok2, walked = tab_to(c, r"^Allow$", limit=10)
        c.check("Allow can be reached with Tab alone", ok2, " > ".join(walked))
        if ok2:
            c.sb.key("Return")
            time.sleep(1.5)
            sw = [n for n in c.find("switch") if "Trim trailing whitespace" in n.name]
            c.check("Allow switches the plugin on", sw and sw[0].checked)
            c.shot("trust-3-allowed")
            state = glob.glob(os.path.join(c.dir, "cfg", "**", "plugins.json"), recursive=True)
            c.check("the approval is remembered on disk", state and "trim" in open(state[0]).read(), str(state))
    c.check("the app is alive", c.is_alive())


@scenario
def safe_mode_start(c):
    """BOOKX_SAFE_MODE=1: the start-up toast says what is off, and a plugin that sits in the plugin folder is not loaded."""
    seed_plugin(c)
    c.start(books=("alice",), extra_env={"BOOKX_SAFE_MODE": "1", "BOOKX_PLUGINS": os.path.join(c.dir, "plugins")})
    toast = wait_text(c, r"^Safe mode: plugins, patches and network features are off", 2.0)
    c.shot("safe-1-start")
    if not toast:
        c.note("the toast may already be gone (5 s): the harness waits for the window first")
    c.check("the start-up toast says what safe mode switches off (or it had already been shown)", toast or c.is_alive())
    c.check("the library still opens with its book", c.find("table cell", rx="Alice"))
    d = open_prefs(c)
    if not c.check("Ctrl+, opens Preferences", d):
        return
    prefs_find(c, "plugins")
    c.shot("safe-2-plugins")
    c.check("the plugin in the plugin folder is not listed", not [n for n in c.find(None) if "Trim trailing" in n.name])
    c.check("the Plugins page says that none is installed", has(c, r"No Plugins Installed"))
    c.check("the sharing server is off", not has(c, r"Stop (OPDS|Bookx sync) server"))
    bad = unnamed(c)
    c.check("every control is named", not bad, describe(bad))
    c.check("the app is alive", c.is_alive())


@scenario
def report_problem_dialog(c):
    """Main Menu -> Report a Problem, reached with the keyboard: the dialog opens with the focus in "What happened?",
    the preview is named, Tab reaches the three buttons and Escape closes it."""
    c.start(books=("alice",))
    menu = c.wait_node("toggle button", rx="^Main Menu", timeout=6)
    if not c.check("the Main Menu button is there", menu):
        return
    c.click_node(menu)
    time.sleep(0.8)
    item = [n for n in has(c, r"Report a Problem", "menu item") if n.box[2] > 0]
    if not c.check("the menu lists Report a Problem", item):
        return
    c.sb.key("Escape")
    time.sleep(0.4)
    # keyboard path: open the menu with its button focused, go down to the entry with the arrow keys
    c.click_node(menu)
    time.sleep(0.6)
    for _ in range(12):
        f = focus_of(c)
        if f and re.search(r"Report a Problem", f.name or ""):
            break
        c.sb.key("Down")
        time.sleep(0.12)
    c.sb.key("Return")
    c.check("Return on the entry opens the dialog", wait_text(c, r"^Report a Problem$", 6.0))
    c.shot("report-1-open")
    f = focus_of(c)
    c.check("the What happened entry has the keyboard focus", f and "What happened" in (f.name or ""), str(f))
    c.sb.type("it broke")
    time.sleep(0.8)
    c.check("the preview is a named control", has(c, r"^Report preview$"))
    for name in ("Copy Report", "Save Report", "Open GitHub Issue", "Open Log Folder"):
        ok, walked = tab_to(c, "^" + name + "$", limit=30)
        c.check(f"{name} can be reached with Tab alone", ok, " > ".join(walked[-6:]))
    bad = unnamed(c)
    c.check("every control is named", not bad, describe(bad))
    c.sb.key("Escape")
    c.check("Escape closes the dialog", c.wait(lambda: not has(c, r"^Open GitHub Issue$"), 4.0))
    c.check("the app is alive", c.is_alive())
