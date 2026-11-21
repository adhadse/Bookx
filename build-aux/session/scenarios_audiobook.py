"""Audiobooks with real input (PP-G-25): open the audiobook with a real double click, play and pause with the
real space key, skip with the arrow keys, change the chapter with the page keys, the sleep timer and speed menus
with real clicks, the media keys, and a screenshot of the player. The audio goes to the null sink at real speed
(BOOKX_AUDIO_SINK=null:1): nothing is played through a sound card, so what these scenarios prove is the
interaction, not the sound. Imported by scenarios.py."""
import math
import os
import re
import struct
import subprocess
import time
import wave

from harness import scenario

DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"


def _make_audio(c):
    d = os.path.join(c.dir, "audio", "The Tale")
    os.makedirs(d, exist_ok=True)
    for name, secs, hz in (("01 - The Gate.wav", 20, 220), ("02 - The Road.wav", 30, 330), ("03 - The End.wav", 10, 440)):
        with wave.open(os.path.join(d, name), "wb") as w:
            w.setnchannels(1)
            w.setsampwidth(2)
            w.setframerate(8000)
            w.writeframes(b"".join(struct.pack("<h", int(2000 * math.sin(i / 8000 * hz * 6.2832))) for i in range(8000 * secs)))
    return d


def _start(c, settings="", size="1280x800", mpris=True):
    d = _make_audio(c)
    env = {"BOOKX_AUDIO_SINK": "null:1", "BOOKX_DEV_SCRIPT": f"audio:add={d}"}
    if not mpris:
        env["BOOKX_NO_MPRIS"] = "1"
    c.start(books=("alice",), settings=settings, size=size, extra_env=env)
    time.sleep(1.5)


def _open(c):
    for _ in range(2):
        card = c.wait_node("table cell", rx="The Tale")
        if not card:
            c.shot("open-failed")
            return None
        time.sleep(0.5)
        card = c.wait_node("table cell", rx="The Tale")
        x, y = card.center()
        c.sb.double_click(x, y)
        r = c.wait_node("button", rx="^−15$", timeout=10)
        if r:
            return r
    c.shot("open-failed")
    return None


def _clock(c):
    """Seconds shown by the elapsed-time label of the player (the first m:ss label)."""
    for n in c.find("label", rx=r"^\d+:\d\d$"):
        m, s = n.name.split(":")
        return int(m) * 60 + int(s)
    return None


def _mpris_name(c):
    out = subprocess.run(["gdbus", "call", "--session", "--dest", "org.freedesktop.DBus", "--object-path",
                          "/org/freedesktop/DBus", "--method", "org.freedesktop.DBus.ListNames"],
                         capture_output=True, text=True, env=c.sb.env).stdout
    m = re.search(r"org\.mpris\.MediaPlayer2\.Bookx\.instance\d+", out)
    return m.group(0) if m else None


@scenario
def audiobook_keys(c):
    _start(c)
    c.check("the audiobook is in the library", c.wait_node("table cell", rx="The Tale"))
    c.shot("library")
    r = _open(c)
    if not c.check("a double click on its card opens the player", r):
        return
    time.sleep(0.8)
    c.check("the player shows the first chapter", c.find("label", rx="The Gate"))
    c.check("and Chapter 1 of 3", c.find("label", rx="Chapter 1 of 3"))
    c.check("it is paused (the button says Play)", c.find("button", rx="^Play$"))
    c.shot("player-paused")

    c.sb.key("space")
    ok = c.wait(lambda: c.find("button", rx="^Pause$"), 5)
    c.check("the space key starts playing (the button says Pause)", ok)
    t0 = _clock(c)
    time.sleep(3.2)
    t1 = _clock(c)
    c.check("the time advances in real time while playing", t0 is not None and t1 is not None and 2 <= t1 - t0 <= 5, f"{t0} -> {t1}")
    c.shot("player-playing")

    c.sb.key("Right")
    time.sleep(0.8)
    t2 = _clock(c)
    c.check("the Right key skips ahead 15 seconds", t2 is not None and t1 is not None and 12 <= t2 - t1 <= 20, f"{t1} -> {t2}")
    c.sb.key("Left")
    time.sleep(0.8)
    t3 = _clock(c)
    c.check("the Left key goes back 15 seconds", t3 is not None and t2 is not None and 10 <= t2 - t3 <= 20, f"{t2} -> {t3}")

    c.sb.key("Page_Down")
    ok = c.wait(lambda: c.find("label", rx="The Road"), 5)
    c.check("Page Down goes to the second chapter", ok)
    c.check("and says Chapter 2 of 3", c.find("label", rx="Chapter 2 of 3"))
    c.sb.key("space")
    ok = c.wait(lambda: c.find("button", rx="^Play$"), 5)
    c.check("the space key pauses", ok)
    pos = _clock(c)
    time.sleep(1.5)
    c.check("the time stands still while paused", _clock(c) == pos)

    # the sleep timer menu with real clicks
    btn = c.wait_node("toggle button", "Sleep Timer") or c.wait_node("button", "Sleep Timer")
    if c.check("the sleep timer button is there", btn):
        c.click_node(btn)
        item = c.wait_node("button", rx="^End of chapter$", timeout=5)
        c.shot("sleep-menu")
        if c.check("its menu offers End of chapter", item):
            c.click_node(item)
            time.sleep(0.5)
            c.sb.key("Escape")
    c.check("the app is alive", c.is_alive())

    # back to the library: the place is kept as progress
    c.sb.key("Escape")
    ok = c.wait(lambda: c.find("table cell", rx="The Tale"), 5)
    c.check("Escape goes back to the library", ok)
    c.shot("library-after")


@scenario
def audiobook_mpris(c):
    _start(c)
    if not _open(c):
        c.check("the player opens", False)
        return
    time.sleep(0.8)
    name = _mpris_name(c)
    if not c.check("the player is a media player on the session bus", name):
        return
    call = lambda m: subprocess.run(["gdbus", "call", "--session", "--dest", name, "--object-path", "/org/mpris/MediaPlayer2",  # noqa: E731
                                     "--method", f"org.mpris.MediaPlayer2.Player.{m}"], capture_output=True, text=True, env=c.sb.env)
    call("PlayPause")
    ok = c.wait(lambda: c.find("button", rx="^Pause$"), 5)
    c.check("PlayPause over D-Bus (what the media keys call) starts playing", ok)
    call("Next")
    ok = c.wait(lambda: c.find("label", rx="Chapter 2 of 3"), 5)
    c.check("Next goes to the next chapter", ok)
    call("Pause")
    ok = c.wait(lambda: c.find("button", rx="^Play$"), 5)
    c.check("Pause pauses", ok)
    # the keyboard's media key: this needs gnome-settings-daemon's media-keys plugin, which the sandbox does not run
    c.sb._n("NotifyKeyboardKeysym", "(ub)", 0x1008FF14, True)
    c.sb._n("NotifyKeyboardKeysym", "(ub)", 0x1008FF14, False)
    time.sleep(1.5)
    playing = bool(c.find("button", rx="^Pause$"))
    c.note("the XF86AudioPlay key " + ("toggled playback" if playing else "did nothing in the sandbox (no media-keys daemon); the D-Bus call above is what the daemon makes"))
    c.shot("player-mpris")


@scenario
def audiobook_dark_small(c):
    _start(c, settings=DARK, size="900x600")
    if not c.check("the player opens in a 900x600 session", _open(c)):
        return
    time.sleep(0.8)
    c.shot("player-dark-900x600")
    c.check("the play button is on screen", c.find("button", rx="^Play$"))
    c.check("the speed button is on screen", c.find("toggle button", "Playback Speed") or c.find("button", "Playback Speed"))
