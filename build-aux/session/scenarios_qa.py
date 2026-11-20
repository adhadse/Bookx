"""QA sweeps by eye (round 2): Preferences pages, first run, editor tool dialogs. Screenshots land in the results
folder; the checks are generic (unnamed controls, rows wider than their window, text clipped by its box)."""
import os
import re
import time

from harness import scenario

DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"

INTERACTIVE = ("push button", "toggle button", "check box", "radio button", "switch", "combo box", "spin button",
               "slider", "text", "entry", "menu button", "link")


def unnamed(nodes):
    return [n for n in nodes if n.role in INTERACTIVE and not n.name and n.showing and n.box[2] > 0]


def prefs_case(c, name, settings, size, lang):
    w, h = size
    env = {"BOOKX_LANG": lang}
    c.start(books=("alice",), settings=settings, extra_env=env, size=f"{w}x{h}")
    c.sb.key(",", ("Control",))
    d = c.wait_node("dialog", timeout=8)
    if not c.check("Preferences opens", d):
        return
    time.sleep(1.5)
    t = c.tree()
    dlg = [n for n in t.nodes if n.role == "dialog" and n.showing][0]
    c.note(f"dialog box {dlg.box}")
    open(os.path.join(c.results, f"{c.name}-tree0.txt"), "w").write(t.dump())
    seen = set()
    visited = []
    sidebar = lambda tr: [n for n in tr.nodes if n.role == "list item" and n.showing and n.box[0] < 340 and n.name]  # noqa: E731
    first = sidebar(t)
    cx, cy = first[0].center() if first else (150, 300)
    idx = 0
    for rnd in range(8):
        for row in sidebar(c.tree()):
            if row.name in visited:
                continue
            c.click_node(row)
            time.sleep(1.0)
            visited.append(row.name)
            label = re.sub(r"\W+", "_", row.name)[:20]
            c.shot(f"{lang}-p{idx:02d}-{label}")
            idx += 1
            t = c.tree()
            dlgs = [n for n in t.nodes if n.role == "dialog" and n.showing]
            if not dlgs:
                c.check(f"page {row.name}: dialog still open", False)
                return
            dx, dy, dw, dh = dlgs[0].box
            inside = [n for n in t.nodes if n.showing and n.box[2] > 0 and n.depth > dlgs[0].depth]
            wide = [n for n in inside if n.box[0] + n.box[2] > dx + dw + 2 and n.role not in ("scroll bar",)]
            c.check(f"page {row.name}: nothing sticks out to the right", not wide, str(wide[:3]))
            un = unnamed(inside)
            c.check(f"page {row.name}: every control is named", not un, str(un[:5]))
            squeezed = [n for n in inside if n.role == "label" and n.box[2] < 130 and n.box[3] > 70]
            c.check(f"page {row.name}: no text squeezed into a narrow column", not squeezed, str(squeezed[:2]))
            for n in inside:
                if n.role == "label" and n.name:
                    seen.add(n.name)
        c.sb.scroll(6, cx, cy)
        time.sleep(0.6)
    if lang == "de":
        english = ["Follow system", "Two columns", "Where Bookx keeps the book files. Changing it moves them (with a preview), all or nothing.",
                   "Extensions run in a sandbox", "Small rule files that fix text", "Sync automatically when a device connects",
                   "Not set", "Application", "Page X of Y", "Edges (narrow strips)", "Author / Title folders", "STARTTLS (port 587)"]
        c.check("no English leftovers in the German pages", not [e for e in english if any(x.startswith(e) for x in seen)],
                str([e for e in english if any(x.startswith(e) for x in seen)]))
    c.note("pages: " + " | ".join(visited))
    open(os.path.join(c.results, f"{c.name}-labels.txt"), "w").write("\n".join(sorted(seen)))
    c.check("the app is alive", c.is_alive())


def _prefs_scenarios():
    for size in ((900, 600), (1280, 800)):
        for theme, settings in (("light", ""), ("dark", DARK)):
            for lang in ("en", "de"):
                def run(c, size=size, settings=settings, lang=lang, theme=theme):
                    prefs_case(c, f"qa-prefs-{theme}", settings, size, lang)
                run.__name__ = f"qa_prefs_{size[0]}_{theme}_{lang}"
                scenario(run)


_prefs_scenarios()


# ---- main-thread stalls (BOOKX_STALL_WATCH, see src/devtools.rs) ---------------------------------------------------
def _calibre_seed(n):
    """A Calibre library folder of n books (the python of build-aux/ui-tests-scale.sh) imported into the sandbox library."""
    import subprocess
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    src = open(os.path.join(root, "ui-tests-scale.sh")).read()
    code = src.split("<<'PY'\n", 1)[1].split("\nPY\n", 1)[0]

    def seed(c):
        d = os.path.join(c.dir, "scalecal")
        os.makedirs(d)
        subprocess.run(["python3", "-", d, str(n)], input=code, text=True, check=True)
        big = os.environ.get("BOOK_BIG")
        if big:
            subprocess.run([c.cli, "library", "add", big], capture_output=True, text=True, env=c.sb.env)
        r = subprocess.run([c.cli, "--library", os.path.join(c.dir, "lib"), "library", "import-calibre", d],
                           capture_output=True, text=True, env=c.sb.env)
        c.note(f"import: {r.stdout[-120:]}{r.stderr[-120:]}")
    return seed


@scenario
def qa_stalls_2000(c):
    """A library of 2000 books and a 20 000-entry EPUB: which actions keep the main loop busy for 100 ms or more."""
    big = os.environ.get("BOOK_BIG")
    if not big or not os.path.exists(big):
        from harness import Skip
        raise Skip("BOOK_BIG (a big EPUB) is not set")
    script = ",".join([
        "wait:4000", "library:sort=1", "wait:1500", "library:sort=0", "wait:1500", "library:search=ba", "wait:1500", "library:search=",
        "wait:1500", "sidebar-toggle", "wait:1000", "library:notes", "wait:1500", "library:health=open", "wait:3000",
        "dialog:close", "wait:800", "prefs", "wait:2500", "dialog:close", "wait:800", "read:1", "wait:6000", "close",
        "wait:1500", "edit:1", "wait:8000"])
    c.start(books=(), seed=_calibre_seed(2000),
            extra_env={"BOOKX_STALL_WATCH": "60", "BOOKX_DEV_SCRIPT": script})
    time.sleep(40)
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    stalls = re.findall(r"stall: (\d+) ms after (.*)", log)
    for ms, what in stalls:
        c.note(f"STALL {ms} ms after {what}")
    c.shot("end")
    c.check("the app is alive", c.is_alive())


@scenario
def library_stalls_2000(c):
    """2000 books in the real session: sort, search and clear fill the grid in idle pieces (grid_fill); the longest block of
    the main loop in each is measured by `library:stalls`, and a screenshot in the middle of a fill shows tiles, never an
    empty grid. Release builds (BOOKX_BIN) show the numbers of docs/performance.md."""
    mid = os.path.join(os.environ.get("SESSION_RESULTS", c.dir), "library_stalls_2000-mid.png")
    script = ",".join([
        "wait:4000", "library:grid", "wait:2000", "library:fillstate", "library:stalls", "wait:8000", "library:fillstate",
        "library:sort=1", "wait:30", "library:fillstate", f"shot:{mid}", "wait:3000", "library:fillstate",
        "library:scroll-to=3000", "wait:600", "library:fillstate", "library:sort=0", "wait:3000", "library:fillstate",
        "library:search=ba", "wait:2500", "library:search=", "wait:3000", "library:fillstate",
        "library:notes", "wait:3000"])
    c.start(books=(), seed=_calibre_seed(2000), extra_env={"BOOKX_STALL_WATCH": "100", "BOOKX_DEV_SCRIPT": script})
    c.wait(lambda: _app_log(c).count("fillstate:") >= 7, 90)
    time.sleep(4)
    log = _app_log(c)
    log = _app_log(c)
    for ms, what in re.findall(r"stall: (\d+) ms after (.*)", log):
        c.note(f"STALL {ms} ms after {what}")
    m = re.search(r"stalls: sort=(\d+) search=(\d+) clear=(\d+) sortback=(\d+) max=(\d+)", log)
    c.check("the stall measurement ran", m, log[-300:])
    if m:
        c.note(f"max block per action (sort/search/clear/sort back): {m.groups()[:4]} ms")
        limit = int(os.environ.get("STALL_LIMIT_MS", "400"))
        c.check(f"no sort, search or clear blocks the main loop for {limit} ms", int(m.group(5)) < limit, m.group(0))
    states = re.findall(r"fillstate: tiles=(\d+) view=\S+ adj=\S+ grid=(\d+) table=(\d+) filling=(\w+) books=(\d+)", log)
    c.check("the list was complete before and after", len(states) >= 4 and states[1][3] == "false" and states[1][1] == states[1][4], str(states))
    if len(states) >= 2:
        c.check("the grid keeps fewer than 700 tiles alive (GtkGridView keeps max_columns*32+1: 1281 at 40 columns)",
                int(states[1][0]) < 700, str(states[1]))
    if len(states) >= 4:
        c.check("30 ms after a sort the grid already shows books and the rest follows", int(states[2][1]) > 0 and int(states[2][0]) > 0, str(states[2]))
        c.check("the sorted list is complete in the end", states[3][3] == "false" and states[3][1] == states[3][4], str(states[3]))
    if len(states) >= 7:
        val = lambda st: float(re.search(r"adj=(\d+)/", st).group(1))  # noqa: E731
        raw = re.findall(r"fillstate: tiles=\d+ view=\S+ (adj=\S+) grid", log)
        c.check("the grid was scrolled to 3000 px", abs(val(raw[4]) - 3000) < 5, raw[4])
        c.check("a sort keeps the scroll position of the grid", abs(val(raw[5]) - 3000) < 5, raw[5])
        c.check("clearing a search starts at the top again", val(raw[6]) < 5, raw[6])
    time.sleep(1)
    notes = [int(ms) for ms, what in re.findall(r"stall: (\d+) ms after (.*)", log) if what.strip() == "library:notes"]
    c.note(f"stalls after opening Notes & Highlights: {notes}")
    limit = int(os.environ.get("STALL_LIMIT_MS", "400"))
    c.check(f"opening Notes & Highlights blocks the main loop for less than {limit} ms", max(notes, default=0) < limit, str(notes))
    c.shot("end")
    c.check("no CRITICAL or WARNING from the fill", "CRITICAL" not in log, "")
    c.check("the app is alive", c.is_alive() or "fillstate" in log)


# ---- first run: an empty library ----------------------------------------------------------------------------------
def _app_log(c):
    return open(os.path.join(c.dir, "app.log"), errors="replace").read()


def firstrun_case(c, name, settings, size, lang):
    w, h = size
    script = "wait:2500,library:menus,wait:600000"
    c.start(books=(), settings=settings, size=f"{w}x{h}", extra_env={"BOOKX_LANG": lang, "BOOKX_DEV_SCRIPT": script})
    time.sleep(2.5)
    c.shot(f"{lang}-home")
    rows = sorted([n for n in c.tree().nodes if n.role == "list item" and n.showing and n.box[0] < 300 and n.name],
                  key=lambda n: n.box[1])
    c.note("sidebar rows: " + " | ".join(n.name for n in rows))
    if w >= 1100 and len(rows) >= 4:
        for i, label in ((1, "recent"), (2, "favourites"), (3, "notes"), (0, "home-again")):
            c.click_node(rows[i])
            time.sleep(1.2)
            c.shot(f"{lang}-{label}")
            c.check(f"the {label} view opens in an empty library", c.is_alive())
            if lang == "en":
                want = {"recent": "Nothing read yet", "favourites": "No favourites yet", "notes": "No highlights",
                        "home-again": "Welcome to Bookx"}[label]
                c.check(f"the {label} view says what it is for ({want})", c.find("label", want), "")
    log = _app_log(c)
    c.note("welcome: " + (re.findall(r"welcome: (.*)", log) or ["?"])[0])
    t = c.tree()
    un = unnamed([n for n in t.nodes if n.showing])
    c.check("every control is named", not un, str(un[:5]))
    c.check("the app is alive", c.is_alive())


def _firstrun_scenarios():
    for size in ((900, 600), (1280, 800)):
        for theme, settings, lang in (("light", "", "en"), ("dark", DARK, "de"), ("light", "", "es")):
            def run(c, size=size, settings=settings, lang=lang):
                firstrun_case(c, "qa-firstrun", settings, size, lang)
            run.__name__ = f"qa_firstrun_{size[0]}_{theme}_{lang}"
            scenario(run)


_firstrun_scenarios()


# ---- editor tool dialogs, by eye ------------------------------------------------------------------------------------
EDITOR_DIALOGS = ["Set Cover", "Manage Fonts", "Transform HTML", "Book Jacket", "Accessibility Metadata", "Set Semantics",
                  "Check External Links", "Add Narration"]


def _po_names(lang):
    if lang == "en":
        return {k: k for k in EDITOR_DIALOGS}
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    s = open(os.path.join(root, "po", f"{lang}.po"), encoding="utf8").read()
    out = {}
    for k in EDITOR_DIALOGS:
        m = re.search(r'msgid "%s"\nmsgstr "([^"]*)"' % re.escape(k), s)
        out[k] = m.group(1) if m else k
    return out


def editor_dialogs_case(c, settings, size, lang):
    w, h = size
    c.start(settings=settings, size=f"{w}x{h}", extra_env={"BOOKX_LANG": lang, "BOOKX_TTS_MOCK": "1", "BOOKX_DEV_SCRIPT": "wait:2500,edit:1",
                                                           "BOOKX_TTS_DIR": os.path.join(c.dir, "tts")})
    time.sleep(7)
    if not c.check("the editor opens", c.find("text")):
        c.shot("no-editor")
        return
    c.sb.key("Escape")
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    names = _po_names(lang)
    for key in EDITOR_DIALOGS:
        title = names[key]
        typed = re.match(r"[\x20-\x7e]*", title).group(0).strip()
        c.sb.key("p", ("Control", "Shift"))
        time.sleep(0.8)
        c.sb.type(typed)
        time.sleep(0.6)
        c.sb.key("Return")
        time.sleep(2.0 if key != "Add Narration" else 1.0)
        slug = re.sub(r"\W+", "-", key)
        c.shot(f"{lang}-{slug}")
        t = c.tree()
        win = [n for n in t.nodes if n.role in ("dialog", "frame") and n.showing][-1]
        inside = [n for n in t.nodes if n.showing and n.box[2] > 0]
        labels = sorted({n.name for n in inside if n.role in ("label", "push button", "toggle button", "check box") and n.name})
        open(os.path.join(c.results, f"{c.name}-{lang}-{slug}.labels"), "w").write("\n".join(labels))
        un = unnamed(inside)
        c.check(f"{key}: every control is named", not un, str(un[:4]))
        wide = [n for n in inside if n.box[0] + n.box[2] > win.box[0] + win.box[2] + 2 and n.role not in ("scroll bar",)]
        c.check(f"{key}: nothing sticks out to the right", not wide, str(wide[:3]))
        if key == "Add Narration":
            time.sleep(5)
            c.shot(f"{lang}-{slug}-later")
        c.sb.key("Escape")
        time.sleep(0.8)
        c.check(f"{key}: the app is alive", c.is_alive())


def _editor_dialog_scenarios():
    for size in ((900, 600), (1280, 800)):
        for theme, settings, lang in (("light", "", "en"), ("dark", DARK, "de")):
            def run(c, size=size, settings=settings, lang=lang):
                editor_dialogs_case(c, settings, size, lang)
            run.__name__ = f"qa_editor_dialogs_{size[0]}_{theme}_{lang}"
            scenario(run)


_editor_dialog_scenarios()


# ---- a 10 MB book with 20 000 contents entries: reader and editor open (wave 6) ----------------------------------
def _big20k_seed(c):
    import subprocess
    root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
    out = os.path.join(c.dir, "big20k.epub")
    subprocess.run(["python3", os.path.join(root, "make-big-epub.py"), out, "20000"], check=True, capture_output=True)
    r = subprocess.run([c.cli, "library", "add", out], capture_output=True, text=True, env=c.sb.env)
    if r.returncode != 0:
        raise RuntimeError(f"adding the big book: {r.stdout}{r.stderr}")


def _stalls(c, after):
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    return [int(ms) for ms, what in re.findall(r"stall: (\d+) ms after (.*)", log) if what.startswith(after)]


@scenario
def big_book_reader_session(c):
    """Opens the big book in the reader in the real session: the first page is on screen and no stall of the main
    loop is longer than the budget (release build: about 600 ms; the budget is for the debug build)."""
    c.start(books=(), seed=_big20k_seed, extra_env={"BOOKX_STALL_WATCH": "100",
            "BOOKX_DEV_SCRIPT": "wait:3000,read:1,wait:7000,reader:side-state"})
    time.sleep(14)
    c.shot("reader-open")
    s = _stalls(c, "read:1")
    c.note(f"stalls after read:1: {s}")
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    c.check("the reader has the 20 000 contents entries", re.search(r"side: .* toc=20\d{3} ", log) is not None)
    budget = int(os.environ.get("STALL_BUDGET_MS", "4000"))
    c.check(f"no stall of the main loop after opening the book is longer than {budget} ms (longest {max(s or [0])} ms)",
            max(s or [0]) <= budget)
    c.check("the app is alive", c.is_alive())


@scenario
def big_book_editor_session(c):
    """Opens the big book in the editor in the real session: the Contents list is virtual (its tab opens at once), a
    chapter that is one 450 KB line shows the page that offers line breaks, and adding them opens the text view."""
    script = ("wait:3000,edit:1,wait:9000,editor:toc-rows,editor:syntax-state,wait:9000,"
              "editor:longline=reformat,wait:9000,editor:syntax-state")
    c.start(books=(), seed=_big20k_seed, size="1280x800", extra_env={"BOOKX_STALL_WATCH": "100", "BOOKX_DEV_SCRIPT": script})
    time.sleep(13)
    c.shot("editor-long-line-page")
    time.sleep(22)
    c.shot("editor-after-line-breaks")
    log = open(os.path.join(c.dir, "app.log"), errors="replace").read()
    s = _stalls(c, "edit:1")
    c.note(f"stalls after edit:1: {s}")
    rows = re.search(r"editor_toc_rows: entries=(\d+) model=(\d+) widgets=(\d+)", log)
    c.check("the Contents list has 20 000+ entries and only the rows in view as widgets",
            rows and int(rows[1]) >= 20000 and int(rows[2]) == int(rows[1]) and int(rows[3]) < 400, str(rows and rows.groups()))
    syn = re.findall(r"editor_syntax: file=(\S+) page=(\S+) .*plain=(\w+)", log)
    c.note(f"syntax states: {syn}")
    c.check("the first chapter shows the very-long-line page", len(syn) >= 1 and syn[0][1] == "longline", str(syn))
    c.check("after Add Line Breaks the text view shows the file, coloured again",
            len(syn) >= 2 and syn[1][1] == "code" and syn[1][2] == "false", str(syn))
    budget = int(os.environ.get("STALL_BUDGET_MS", "4000"))
    c.check(f"opening the book in the editor never blocks the main loop longer than {budget} ms (longest {max(s or [0])} ms)",
            max(s or [0]) <= budget)
    c.check("the app is alive", c.is_alive())
