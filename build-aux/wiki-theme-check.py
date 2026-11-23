#!/usr/bin/env python3
"""Checks in a real browser that the colour-scheme toggle of the built wiki swaps the screenshots.

  wiki-theme-check.py SITE_DIR [--shots DIR]

Serves SITE_DIR, starts headless Firefox (Marionette, standard library only), and on several pages: (1) with the light
palette only `#only-light` pictures are visible, (2) a real click on the palette toggle in the header switches to the dark
palette and then only `#only-dark` pictures are visible, (3) and back. With --shots DIR it also saves screenshots of the
pages in both palettes at desktop and phone widths. Exits 0 without checking when Firefox is not installed.
"""
import argparse
import base64
import functools
import http.server
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time

PAGES = ["", "library/", "reading/", "read-aloud/", "editing/", "accessibility/"]

# the demo videos (assets/videos.js): the visible twin plays by itself, the hidden one never does
VIDEOS = """
const vs = [...document.querySelectorAll('video[data-clip]')];
const vis = (v) => { const r = v.getBoundingClientRect(); return getComputedStyle(v).display !== 'none' && r.width > 0; };
const inview = (v) => { const r = v.getBoundingClientRect(); return r.top < innerHeight * 0.65 && r.bottom > innerHeight * 0.35; };
return vs.map(v => ({cls: v.className, visible: vis(v), inview: inview(v), paused: v.paused, t: v.currentTime, ready: v.readyState,
                     rm: matchMedia('(prefers-reduced-motion: reduce)').matches, src: v.currentSrc.split('/').pop(), error: v.error ? v.error.code : 0}));
"""
SCROLL_VIDEO = "const vs = [...document.querySelectorAll('video[data-clip]')].filter(v => getComputedStyle(v).display !== 'none'); " \
               "if (vs[arguments[0]]) vs[arguments[0]].scrollIntoView({block: 'center'}); return vs.length;"

VISIBLE = """
const vis = (img) => { const r = img.getBoundingClientRect(); return getComputedStyle(img).display !== 'none' && r.width > 0; };
const imgs = [...document.querySelectorAll('img')];
return {scheme: document.body.getAttribute('data-md-color-scheme'),
        light: imgs.filter(i => /#only-light$/.test(i.getAttribute('src')) && vis(i)).length,
        dark: imgs.filter(i => /#only-dark$/.test(i.getAttribute('src')) && vis(i)).length,
        lightAll: imgs.filter(i => /#only-light$/.test(i.getAttribute('src'))).length,
        darkAll: imgs.filter(i => /#only-dark$/.test(i.getAttribute('src'))).length,
        plain: imgs.filter(i => /-(light|dark)\\.(png|webp)/.test(i.getAttribute('src')) && !/#only-/.test(i.getAttribute('src'))).length};
"""

# click the header button that is for the other palette (the toggle the visitor uses)
TOGGLE = """
const label = [...document.querySelectorAll('.md-header [data-md-component=palette] label:not([hidden])')][0];
if (!label) return 'no toggle';
label.click(); return 'clicked';
"""

SCROLL_TO = "window.scrollTo(0, arguments[0]); return document.documentElement.scrollHeight;"


class Marionette:
    def __init__(self, port):
        self.s = socket.create_connection(("127.0.0.1", port), timeout=60)
        self.n = 0
        self._read()  # hello
        self.cmd("WebDriver:NewSession", {"capabilities": {}})

    def _read(self):
        buf = b""
        while b":" not in buf:
            buf += self.s.recv(1)
        n = int(buf.split(b":")[0])
        data = b""
        while len(data) < n:
            data += self.s.recv(n - len(data))
        return json.loads(data)

    def cmd(self, name, params=None):
        self.n += 1
        body = json.dumps([0, self.n, name, params or {}]).encode()
        self.s.sendall(str(len(body)).encode() + b":" + body)
        while True:
            m = self._read()
            if m[0] == 1 and m[1] == self.n:
                if m[2]:
                    raise RuntimeError(f"{name}: {m[2]}")
                return m[3]

    def script(self, js):
        r = self.cmd("WebDriver:ExecuteScript", {"script": js, "args": []})
        return r.get("value") if isinstance(r, dict) else r

    def load_lazy_images(self):
        """Scrolls down the page in steps so that loading="lazy" pictures are fetched, then back to the top."""
        y, h = 0, 1
        while y < h:
            r = self.cmd("WebDriver:ExecuteScript", {"script": SCROLL_TO, "args": [y]})
            h = r["value"]
            y += 500
            time.sleep(0.12)
        self.cmd("WebDriver:ExecuteScript", {"script": SCROLL_TO, "args": [0]})
        time.sleep(0.4)

    def shot(self, path):
        r = self.cmd("WebDriver:TakeScreenshot", {"full": True})
        with open(path, "wb") as f:
            f.write(base64.b64decode(r["value"]))


def check_videos(m, name, scheme, reduced, errors):
    """Every visible clip of the page in turn: scroll it into view, wait, see whether it plays (see video_problems)."""
    n = m.cmd("WebDriver:ExecuteScript", {"script": SCROLL_VIDEO, "args": [0]})["value"]
    for i in range(n):
        m.cmd("WebDriver:ExecuteScript", {"script": SCROLL_VIDEO, "args": [i]})
        time.sleep(2.5)
        first = m.script(VIDEOS)
        time.sleep(1.2)
        second = m.script(VIDEOS)
        errors.extend(video_problems(name, scheme, first, second, reduced))
        for v, w in zip(first, second):
            if v["visible"] and v["inview"]:
                print(f"{name}: {scheme} palette: video {w['src']} paused={w['paused']} currentTime {v['t']:.2f} -> {w['t']:.2f}")
    m.script("window.scrollTo(0, 0);")


def video_problems(name, scheme, first, second, reduced=False):
    """The visible twin must play (not paused, currentTime advancing); the hidden one must not; with reduced motion neither."""
    out = []
    for a, b in zip(first, second):
        who = f"{name}: {scheme} video {a['src'] or a['cls']}"
        if reduced:
            if not b["paused"] or b["t"] > 0.05:
                out.append(f"{who} plays although the system asks for reduced motion {b}")
        elif b["visible"] and b["inview"]:
            if b["error"]:
                out.append(f"{who} cannot be played (error {b['error']})")
            elif b["paused"] or b["t"] <= a["t"]:
                out.append(f"{who} is not playing by itself {a['t']:.2f} -> {b['t']:.2f}, paused={b['paused']}")
        elif not b["paused"]:
            out.append(f"{who} is hidden or scrolled out of view but plays")
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("site")
    ap.add_argument("--shots")
    ap.add_argument("--reduced-motion", action="store_true", help="run with prefers-reduced-motion: reduce (no video may play)")
    a = ap.parse_args()
    firefox = shutil.which("firefox")
    if not firefox:
        print("firefox not found: skipping the browser check of the light/dark pictures")
        return 0
    class Quiet(http.server.SimpleHTTPRequestHandler):
        def log_message(self, *args):
            pass

    handler = functools.partial(Quiet, directory=os.path.abspath(a.site))
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    base = f"http://127.0.0.1:{httpd.server_address[1]}/"
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        mport = s.getsockname()[1]
    prof = tempfile.mkdtemp(prefix="wiki-theme-")
    with open(os.path.join(prof, "user.js"), "w") as f:
        f.write(f'user_pref("marionette.port", {mport});\nuser_pref("browser.shell.checkDefaultBrowser", false);\n'
                'user_pref("layout.css.prefers-color-scheme.content-override", 1);\n'  # start light
                + ('user_pref("ui.prefersReducedMotion", 1);\n' if a.reduced_motion else ''))
    proc = subprocess.Popen([firefox, "--headless", "--marionette", "--no-remote", "--profile", prof],
                            stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
    errors = []
    try:
        for _ in range(300):
            try:
                m = Marionette(mport)
                break
            except OSError:
                time.sleep(0.2)
        else:
            print("could not start Firefox")
            return 1
        m.cmd("WebDriver:SetWindowRect", {"width": 1280, "height": 900})
        if a.shots:
            os.makedirs(a.shots, exist_ok=True)
        for page in PAGES:
            m.cmd("WebDriver:Navigate", {"url": base + page})
            time.sleep(0.8)
            m.load_lazy_images()
            time.sleep(0.8)
            name = page.strip("/") or "home"
            st = m.script(VISIBLE)
            if st["lightAll"] == 0:
                continue
            check_videos(m, name, "light", a.reduced_motion, errors)
            m.script("window.scrollTo(0, 0);")
            ok = st["scheme"] == "default" and st["light"] == st["lightAll"] and st["dark"] == 0
            print(f"{name}: light palette: {st['light']}/{st['lightAll']} light pictures visible, {st['dark']} dark")
            if not ok:
                errors.append(f"{name}: light palette shows the wrong pictures {st}")
            if a.shots:
                m.shot(os.path.join(a.shots, f"{name}-desktop-light.png"))
            if m.script(TOGGLE) != "clicked":
                errors.append(f"{name}: no palette toggle in the header")
                continue
            time.sleep(0.6)
            m.load_lazy_images()
            check_videos(m, name, "dark", a.reduced_motion, errors)
            m.script("window.scrollTo(0, 0);")
            st = m.script(VISIBLE)
            ok = st["scheme"] == "slate" and st["dark"] == st["darkAll"] and st["light"] == 0
            print(f"{name}: after the toggle ({st['scheme']}): {st['dark']}/{st['darkAll']} dark visible, {st['light']} light")
            if not ok:
                errors.append(f"{name}: dark palette shows the wrong pictures {st}")
            if a.shots:
                m.shot(os.path.join(a.shots, f"{name}-desktop-dark.png"))
                m.cmd("WebDriver:SetWindowRect", {"width": 420, "height": 900})
                time.sleep(0.8)
                m.load_lazy_images()
                time.sleep(0.6)
                m.shot(os.path.join(a.shots, f"{name}-phone-dark.png"))
            m.script(TOGGLE)  # back to light
            time.sleep(0.6)
            m.load_lazy_images()
            st = m.script(VISIBLE)
            if st["scheme"] != "default" or st["dark"] != 0:
                errors.append(f"{name}: toggling back did not restore the light pictures {st}")
            if a.shots:
                m.shot(os.path.join(a.shots, f"{name}-phone-light.png"))
                m.cmd("WebDriver:SetWindowRect", {"width": 1280, "height": 900})
    finally:
        proc.terminate()
        try:
            proc.wait(10)
        except subprocess.TimeoutExpired:
            proc.kill()
        httpd.shutdown()
        shutil.rmtree(prof, ignore_errors=True)
    for e in errors:
        print("ERROR", e)
    print(f"{len(errors)} problems")
    return 1 if errors else 0


if __name__ == "__main__":
    sys.exit(main())
