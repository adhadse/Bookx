"""Short videos for the public wiki (build-aux/wiki-screenshots.sh, build-aux/wiki_video.py): read aloud, speed reading and the
reading ruler. A video is a series of real screenshots taken while the feature runs, with the time of every one; wiki_video.py
encodes them. Same rules as scenarios_wiki.py (WIKI_SCHEME=light|dark, no focus rings, no pointer in the frames)."""
import json
import os
import time

from harness import scenario, Skip  # noqa: F401
from scenarios_wiki_ext import reader_start, click, ui  # noqa: F401

PANEL = 32  # the GNOME top bar, cut like in the framed pictures
MAX_FRAMES = 160


def frame_path(results, scene, name, i):
    return os.path.join(results, f"{scene}-{name}.f{i:04d}.frame")


def record(c, name, seconds, acts=(), pause=0.0):
    """Takes screenshots (the window without the top bar, no pointer) for `seconds`; `acts` is a list of (second, function)
    run when the clip reaches that second. Writes <scene>-<name>.fNNNN.frame and <scene>-<name>.timing.json."""
    from PIL import Image
    c.sb.move_to(c.sb.w // 2, 10)
    time.sleep(0.4)
    tmp = os.path.join(c.dir, "video-tmp.png")
    acts = sorted(acts, key=lambda a: a[0])
    times = []
    t0 = time.monotonic()
    while len(times) < MAX_FRAMES:
        t = time.monotonic() - t0
        if t >= seconds:
            break
        while acts and acts[0][0] <= t:
            acts.pop(0)[1]()
        c.sb.screenshot_to(tmp, cursor=False)
        img = Image.open(tmp).convert("RGB")
        img = img.crop((0, PANEL, img.width, img.height))
        if img.width > 960:
            img = img.resize((960, round(img.height * 960 / img.width)), Image.LANCZOS)
        i = len(times) + 1
        img.save(frame_path(c.results, c.name, name, i), "PNG")
        times.append(round(t, 3))
        if pause:
            time.sleep(pause)
    with open(os.path.join(c.results, f"{c.name}-{name}.timing.json"), "w") as f:
        json.dump(times, f)
    c.note(f"{name}: {len(times)} frames in {times[-1] if times else 0:.1f} s")
    return times


@scenario
def _wiki_video_aloud(c):
    """Read aloud: the sentence being read is highlighted and moves on (a silent mock engine, 600 ms per sentence)."""
    reader_start(c, env={"BOOKX_TTS_MOCK": "1100"})
    btn = c.wait_node("toggle button", rx="^Read Aloud")
    if not c.check("the Read aloud button", btn):
        return
    c.click_node(btn)
    play = c.wait_node("button", "Read Aloud from Here", timeout=6)
    if not c.check("the popover has a play button", play):
        return
    c.click_node(play)
    c.sb.key("Escape")
    c.wait_node("button", "Next Sentence", timeout=6)

    times = record(c, "read-aloud-demo", 10.0)
    c.check("frames of read aloud", len(times) >= 12, str(len(times)))


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
    c.click_node(start)
    times = record(c, "speed-read-demo", 8.0)
    c.check("frames of speed reading", len(times) >= 12, str(len(times)))


@scenario
def _wiki_video_ruler(c):
    """The reading ruler: Alt+Down turns it on and moves the band down line by line, Alt+Up back."""
    reader_start(c)

    def down():
        c.sb.key("Down", ("Alt",))

    def up():
        c.sb.key("Up", ("Alt",))
    acts = [(0.5, down)] + [(1.3 + 0.7 * i, down) for i in range(7)] + [(7.0 + 0.7 * i, up) for i in range(3)]
    times = record(c, "reading-ruler-demo", 10.0, acts)
    c.check("frames of the ruler", len(times) >= 12, str(len(times)))
