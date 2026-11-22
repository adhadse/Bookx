"""Videos for the public wiki (build-aux/wiki-screenshots.sh, build-aux/wiki_video.py): read aloud, speed reading and the reading
ruler. Each is a REAL screen recording of the sandboxed GNOME session (build-aux/session/screencast.py: Mutter's screen cast
through PipeWire and GStreamer, 30 fps, no pointer), not a series of screenshots. Same rules as scenarios_wiki.py
(WIKI_SCHEME=light|dark, no focus rings)."""
import os
import time

from harness import scenario, Skip  # noqa: F401
from scenarios_wiki_ext import reader_start, click  # noqa: F401
from screencast import Screencast

FPS = 30


def record(c, name, seconds, acts=()):
    """Records the window (without the GNOME top bar) for `seconds` while `acts`, a list of (second, function), run at their
    time. Writes <scene>-<name>.raw.mkv into the results folder (wiki_video.py encodes it) and returns the length."""
    c.sb.move_to(c.sb.w // 2, 10)
    time.sleep(0.4)
    out = os.path.join(c.results, f"{c.name}-{name}.raw.mkv")
    rec = Screencast(c, area=(0, 32, c.sb.w, c.sb.h - 32), fps=FPS)
    try:
        rec.start(out)
        t0 = time.monotonic()
        for at, act in sorted(acts, key=lambda a: a[0]):
            time.sleep(max(0.0, at - (time.monotonic() - t0)))
            act()
        time.sleep(max(0.0, seconds - (time.monotonic() - t0)))
    finally:
        rec.stop()
    size = os.path.getsize(out) if os.path.exists(out) else 0
    c.note(f"{name}: recorded {time.monotonic() - t0:.1f} s, {size // 1024} KB (raw)")
    return size


@scenario
def _wiki_video_aloud(c):
    """Read aloud: the sentence being read is highlighted and moves on (a silent mock engine)."""
    reader_start(c, env={"BOOKX_TTS_MOCK": "1100"})
    btn = c.wait_node("toggle button", rx="^Read Aloud")
    if not c.check("the Read aloud button", btn):
        return
    c.click_node(btn)
    play = c.wait_node("button", "Read Aloud from Here", timeout=6)
    if not c.check("the popover has a play button", play):
        return

    def start_reading():
        c.click_node(play)
        c.sb.key("Escape")
    size = record(c, "read-aloud-demo", 11.0, [(0.8, start_reading)])
    c.check("a recording of read aloud", size > 20000, str(size))


@scenario
def _wiki_video_speed(c):
    """Speed reading: one word at a time on the page, started from the Main Menu."""
    reader_start(c)
    click(c, "toggle button", "Main Menu")
    items = c.wait(lambda: sorted([n for n in c.find("menu item") if n.showing and n.box[2] > 0],
                                  key=lambda n: n.box[1]) or None, 5)
    item = items[1] if items and len(items) > 1 else None
    if not c.check("the Speed Reading menu item", item):
        return
    c.click_node(item)
    time.sleep(1.0)
    start = c.wait_node("button", "Start", timeout=6)
    if not c.check("the Start button", start):
        return
    size = record(c, "speed-read-demo", 9.0, [(0.8, lambda: c.click_node(start))])
    c.check("a recording of speed reading", size > 20000, str(size))


def glide(c, x, y0, y1, seconds):
    """Moves the (hidden) pointer smoothly from y0 to y1; the ruler band follows it."""
    t0 = time.monotonic()
    while True:
        f = (time.monotonic() - t0) / seconds
        if f >= 1:
            break
        c.sb.move_to(x, int(y0 + (y1 - y0) * f))
    c.sb.move_to(x, y1)


@scenario
def _wiki_video_ruler(c):
    """The reading ruler: Alt+Down turns it on and moves the band down line by line, Alt+Up back, then the band follows the
    pointer as it glides down the page and up again."""
    reader_start(c)

    def down():
        c.sb.key("Down", ("Alt",))

    def up():
        c.sb.key("Up", ("Alt",))
    acts = ([(0.8, down)] + [(1.5 + 0.55 * i, down) for i in range(5)] + [(4.4 + 0.55 * i, up) for i in range(2)]
            + [(5.8, lambda: glide(c, 320, 130, 560, 3.2)), (9.1, lambda: glide(c, 320, 560, 160, 2.2))])
    size = record(c, "reading-ruler-demo", 11.6, acts)
    c.check("a recording of the ruler", size > 20000, str(size))
