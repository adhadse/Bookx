"""Scenario runner, started inside `dbus-run-session` by build-aux/session-tests.sh.

    run_scenario.py NAME   # one scenario, one private GNOME session

A scenario is a function `def name(c)` registered in scenarios.py; `c` is a `Ctx`.
Results are lines `ok|FAIL|SKIP: text` in <results>/<name>.txt and a screenshot per step.
"""
import os
import re
import subprocess
import sys
import time
import traceback

from PIL import Image

import atspi
from sandbox import Sandbox

SCENARIOS = {}


def scenario(fn=None, *, net=False):
    def reg(f):
        f.net = net
        SCENARIOS[f.__name__] = f
        return f
    return reg(fn) if fn else reg


class Skip(Exception):
    pass


EXTERNAL_KILL = 75  # exit code of run_scenario.py: the app was killed by a signal from outside, run it again
REGISTRY_FAILURE = 76  # the AT-SPI registry did not start (infrastructure, not the app): run it again in a fresh session

# What GTK or the a11y bus logs when the registry could not be started (seen once in a real run:
# "Could not activate remote peer 'org.a11y.atspi.Registry'").
REGISTRY_ERRORS = re.compile(
    r"Could not activate remote peer.{0,20}org\.a11y\.atspi\.Registry"
    r"|org\.a11y\.atspi\.Registry.{0,80}(was not provided|not activatable|Timeout|Failed|failed)"
    r"|at-spi2-registryd.{0,60}(failed|error|Error|Cannot)",
    re.S)


def registry_failure_in(text):
    """Whether log text shows the AT-SPI registry failing to start (pure; tested by selftest.py)."""
    return bool(REGISTRY_ERRORS.search(text))


class RegistryFailure(Exception):
    pass


class Ctx:
    def __init__(self, name, workdir, bin_path, cli, books, results):
        self.name = name
        self.dir = workdir
        self.bin = bin_path
        self.cli = cli
        self.books = books
        self.results = results
        self.log = open(os.path.join(results, f"{name}.txt"), "w")
        self.failed = 0
        self.sb = None
        self.app = None

    # ---- reporting --------------------------------------------------------------------------
    def _say(self, kind, text):
        line = f"{kind}: {text}"
        print(f"[{self.name}] {line}", flush=True)
        self.log.write(line + "\n")
        self.log.flush()

    def check(self, text, cond, detail=""):
        if cond:
            self._say("ok", text)
        else:
            self.failed += 1
            self._say("FAIL", text + (f"  ({detail})" if detail else ""))
        return bool(cond)

    def note(self, text):
        self._say("info", text)

    def shot(self, label):
        self.sb.screenshot(f"{self.name}-{label}")
        return os.path.join(self.results, f"{self.name}-{label}.png")

    def image(self, label=None):
        """The screen as a PIL image (and saved as a result when a label is given)."""
        if label:
            return Image.open(self.shot(label)).convert("RGB")
        p = os.path.join(self.dir, "tmp.png")
        self.sb.screenshot_to(p, cursor=False)
        return Image.open(p).convert("RGB")

    # ---- session ----------------------------------------------------------------------------
    def make_dictionary(self):
        """A StarDict with every plain word of the test book: "Test definition of <word>."."""
        import re
        import struct
        import zipfile
        words = set()
        with zipfile.ZipFile(self.books["alice"]) as z:
            for n in z.namelist():
                if n.endswith((".xhtml", ".html")):
                    t = re.sub(r"<[^>]+>", " ", z.read(n).decode("utf8", "ignore"))
                    words.update(w.lower() for w in re.findall(r"[A-Za-z]{2,}", t))
        d = os.path.join(self.dir, "dicts", "testdict")
        os.makedirs(d, exist_ok=True)
        idx, dic = bytearray(), bytearray()
        for w in sorted(words):
            df = f"Test definition of {w}.".encode()
            idx += w.encode() + b"\0" + struct.pack(">II", len(dic), len(df))
            dic += df
        open(os.path.join(d, "t.dict"), "wb").write(dic)
        open(os.path.join(d, "t.idx"), "wb").write(idx)
        open(os.path.join(d, "t.ifo"), "w").write(
            "StarDict's dict ifo file\nversion=2.4.2\nwordcount=%d\nidxfilesize=%d\n"
            "bookname=Session Test Dict\nsametypesequence=m\n" % (len(words), len(idx)))

    def start(self, books=("alice",), extra_env=None, settings="", size="1280x800"):
        self.make_dictionary()
        self.sb = Sandbox(self.dir, size=size, settings=settings)
        self.sb.results = self.results
        self.sb.start()
        for b in books:
            r = subprocess.run([self.cli, "library", "add", self.books[b]], capture_output=True, text=True,
                               env=self.sb.env)
            if r.returncode != 0:
                raise RuntimeError(f"importing {b}: {r.stdout}{r.stderr}")
        # BOOKX_DEVICE_ROOTS: without it the app lists the e-readers that are mounted on THIS machine
        env = {"BOOKX_LIBRARY": os.path.join(self.dir, "lib"),
               "BOOKX_DICTIONARIES": os.path.join(self.dir, "dicts"),
               "BOOKX_DEVICE_ROOTS": os.path.join(self.dir, "mounts")}
        if os.environ.get("WAYLAND_DEBUG"):  # protocol trace of the app, for hunting compositor kills
            env["WAYLAND_DEBUG"] = os.environ["WAYLAND_DEBUG"]
        # automatic dictionary downloads never go to the network by themselves in the tests
        env["BOOKX_DICT_AUTO"] = "off"
        env.update(extra_env or {})
        cmd = [self.bin]
        if os.environ.get("GDB") == "1":  # debugging aid: a backtrace of a crash lands in app.log
            cmd = ["gdb", "-batch", "-ex", "run", "-ex", "bt 25", "--args", self.bin]
        self.app_cmd, self.app_env = cmd, env
        self.app = self.sb.launch(cmd, env)
        if not self.wait(lambda: self.tree().find("frame"), 25):
            if self.registry_broken():
                raise RegistryFailure("the AT-SPI registry did not start")
            raise RuntimeError("the app window never showed up (see app.log)")
        time.sleep(1.5)

    def restart_app(self):
        """Quits the app (SIGTERM, like closing it from outside) and starts it again in the same sandbox: same
        library, config and state folders, a new process."""
        self.app.terminate()
        try:
            self.app.wait(10)
        except subprocess.TimeoutExpired:
            self.app.kill()
            self.app.wait(5)
        self.app = self.sb.launch(self.app_cmd, self.app_env, log="app2.log")
        if not self.wait(lambda: self.tree().find("frame"), 25):
            raise RuntimeError("the app window never showed up after the restart (see app2.log)")
        time.sleep(1.5)

    def stop(self):
        if self.sb:
            self.sb.stop()

    def registry_broken(self):
        """The AT-SPI registry failed to start in this session (see the logs of the app, the shell and the registry)."""
        for name in ("app.log", "shell.log", "atspi-registry.log"):
            try:
                with open(os.path.join(self.dir, name), errors="replace") as f:
                    if registry_failure_in(f.read()):
                        return True
            except OSError:
                pass
        return False

    # ---- inspection -------------------------------------------------------------------------
    def tree(self, hidden=False):
        return atspi.Tree("bookx", include_hidden=hidden)

    def wait(self, fn, timeout=10.0, every=0.3):
        return atspi.wait_for(fn, timeout, every)

    def find(self, role=None, name=None, rx=None):
        return self.tree().find(role, name, rx)

    def wait_node(self, role=None, name=None, rx=None, timeout=10.0):
        r = self.wait(lambda: self.find(role, name, rx), timeout)
        return r[0] if r else None

    def gone(self, role=None, name=None, rx=None, timeout=5.0):
        return bool(self.wait(lambda: not self.find(role, name, rx), timeout))

    def click_node(self, node, **kw):
        x, y = node.center()
        self.sb.click(x, y, **kw)

    def edit_book_from_menu(self):
        """In the open book context menu: Edit (3rd entry) ▸ Edit Book (3rd entry of its page, after the
        back button and Edit Metadata). Menu items have no accessible names, so this goes by position."""
        self.click_node(self.find("menu item")[2])
        time.sleep(0.5)
        self.click_node(self.find("menu item")[2])

    def is_alive(self):
        return self.app.poll() is None

    def killed_from_outside(self):
        """SIGTERM/SIGKILL/SIGINT: someone ran `pkill bookx` (another test run, ./run.sh), the app did not crash."""
        rc = self.app.poll() if self.app else None
        return rc in (-15, -9, -2)

    # ---- reader helpers ---------------------------------------------------------------------
    def open_book(self, title_rx="Alice"):
        """Double-click the book's card (again once, if the grid was still being laid out)."""
        for attempt in range(2):
            card = self.wait_node("table cell", rx=title_rx)
            if not card:
                raise RuntimeError("no book card in the library")
            time.sleep(0.5)
            card = self.wait_node("table cell", rx=title_rx)
            x, y = card.center()
            self.sb.double_click(x, y)
            r = self.wait_node("slider", timeout=12)
            if r:
                return r
        return None

    def page_label(self):
        """'N of M' from the reader's footer label, or None."""
        import re
        for n in self.find("label", rx=r"^\d+ of \d+(?:  ·.*)?$"):
            return tuple(int(v) for v in re.match(r"(\d+) of (\d+)", n.name).groups())
        return None

    def go_to_text(self, page=8):
        """Page forward with the keyboard until the footer says `page` (the first pages are the cover and title)."""
        for _ in range(page + 4):
            p = self.page_label()
            if p and p[0] >= page:
                break
            self.sb.key("Right")
            time.sleep(0.5)
        time.sleep(1.2)
        return self.page_label()

    def ocr(self, img, box=None):
        """Text in a screen region (tesseract). Used where GTK exposes no accessible text: menu items and the page canvas."""
        import shutil
        if not shutil.which("tesseract"):
            return None
        if box:
            img = img.crop(box)
        img = img.resize((img.width * 2, img.height * 2))
        p = os.path.join(self.dir, "ocr.png")
        img.save(p)
        r = subprocess.run(["tesseract", p, "-", "--psm", "6"], capture_output=True, text=True)
        return r.stdout

    @staticmethod
    def count_pixels(img, box, pred):
        x0, y0, x1, y1 = box
        n = 0
        px = img.load()
        for y in range(y0, min(y1, img.height)):
            for x in range(x0, min(x1, img.width)):
                if pred(*px[x, y]):
                    n += 1
        return n

    def screen_box(self, node):
        x, y, w, h = node.box
        ox, oy = atspi.ORIGIN
        return (x + ox, y + oy, x + ox + w, y + oy + h)

    def glyph_near(self, x, y, img=None, radius=24):
        """A pixel on a letter close to (x, y): double-clicking a space would select nothing."""
        img = img or self.image()
        w, h = img.size
        best = None
        for r in range(0, radius):
            for dx in range(-r, r + 1):
                for dy in (-r, r):
                    for (px, py) in ((x + dx, y + dy), (x + dy, y + dx)):
                        if 0 <= px < w and 0 <= py < h:
                            R, G, B = img.getpixel((px, py))
                            if R + G + B < 200:
                                # make sure it is not a lone speck: some more dark pixels around it
                                dark = sum(1 for ddx in range(-3, 4) for ddy in range(-3, 4)
                                           if 0 <= px + ddx < w and 0 <= py + ddy < h
                                           and sum(img.getpixel((px + ddx, py + ddy))) < 300)
                                if dark >= 6:
                                    best = (px, py)
                                    return best
        return best


def run(name):
    import scenarios  # noqa: F401  (registers)
    fn = SCENARIOS[name]
    root = os.environ["SESSION_TEST_DIR"]
    # never let a test touch a real library or config
    for var in ("BOOKX_LIBRARY", "HOME", "XDG_CONFIG_HOME", "XDG_RUNTIME_DIR"):
        if not os.environ.get(var, "").startswith(root):
            sys.exit(f"refusing to run: {var} is not inside the sandbox {root}")
    c = Ctx(name, root, os.environ["BOOKX_BIN"], os.environ["BOOKX_CLI"],
            {"alice": os.environ["BOOK_ALICE"], "tiny": os.environ["BOOK_TINY"],
             "vsynth": os.environ.get("BOOK_VSYNTH", ""), "vneko": os.environ.get("BOOK_VNEKO", ""), "pdf": os.environ.get("BOOK_PDF", "")},
            os.environ["SESSION_RESULTS"])
    code = 0
    killed = False
    registry = False
    try:
        if fn.net and os.environ.get("NET") == "0":
            raise Skip("NET=0")
        fn(c)
        if c.app and not c.is_alive():
            c.check("the app is still running at the end", False, f"exit status {c.app.returncode}")
    except Skip as e:
        c._say("SKIP", str(e))
    except RegistryFailure:
        registry = True
    except Exception:
        registry = c.registry_broken()
        c.failed += 1
        c._say("FAIL", "scenario crashed:\n" + traceback.format_exc())
        try:
            c.shot("crash")
        except Exception:
            pass
    finally:
        killed = bool(c.app) and c.killed_from_outside()
        try:
            c.stop()
        except Exception:
            pass
    if registry:
        c._say("RETRY", "the AT-SPI registry did not start (infrastructure); run again in a fresh session")
        sys.exit(REGISTRY_FAILURE)
    if killed:
        c._say("RETRY", "the app was killed by a signal from outside (another process ran pkill?); run again")
        sys.exit(EXTERNAL_KILL)
    code = 1 if c.failed else 0
    c._say("done", f"{name}: {'FAILED' if c.failed else 'passed'}")
    sys.exit(code)
