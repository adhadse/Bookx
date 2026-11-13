"""Read aloud in the real GNOME session: real pointer and keyboard on the popover, AT-SPI names, screenshots.
Imported by scenarios.py. The silent mock engine (BOOKX_TTS_MOCK, 400 ms per sentence) stands in for the voice,
so nothing needs a model or a sound card; what is checked is the flow the user sees."""
import time

import atspi
from harness import scenario


def tinted(img):
    """Pixels of the page painted with the (bluish) highlight of the sentence being read."""
    px = img.load()
    return sum(1 for x in range(100, min(img.width, 1180), 2) for y in range(100, 700, 2)
               if px[x, y][2] - px[x, y][0] > 30 and px[x, y][0] > 150)


@scenario
def read_aloud_flow(c):
    """Open the popover, play, skip, pause, change speed, stop; the settings button opens Preferences on the
    Text to Speech page."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "400"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    btn = c.wait_node("toggle button", rx="^Read aloud")
    if not c.check("the reader has a Read aloud button", btn):
        return
    c.click_node(btn)
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens with a play button", play):
        return
    for name, role in (("Previous sentence", "button"), ("Next sentence", "button"), ("Stop reading", "button"),
                       ("Reading speed", "slider"), ("Pitch", "slider")):
        c.check(f"the popover has {name}", c.find(role, name))
    c.check("and the sleep timer", c.find(None, "Sleep timer"))
    before = c.image("tts-before")
    c.check("nothing is highlighted before reading", tinted(before) < 30, str(tinted(before)))
    c.click_node(play)
    paused = c.wait_node("button", "Pause reading", timeout=6)
    c.check("play turns into a pause button", paused)
    c.sb.key("Escape")  # close the popover to see the page
    time.sleep(1.6)
    during = c.image("tts-playing")
    c.check("the sentence being read is highlighted on the page", tinted(during) > 300, str(tinted(during)))
    # the player card floats over the page: real clicks on its own buttons
    nxt = c.wait_node("button", "Next sentence", timeout=5)
    c.check("the player card is on the page after the popover closed", nxt)
    c.click_node(nxt)
    time.sleep(0.8)
    c.check("skipping keeps reading", c.find("button", "Pause reading"))
    c.click_node(c.find("button", "Pause reading")[0])
    c.check("pause turns into resume", c.wait_node("button", "Resume reading", timeout=4))
    c.click_node(c.find("button", "Resume reading")[0])
    c.check("resume reads on", c.wait_node("button", "Pause reading", timeout=4))
    c.click_node(c.find("button", "Close read aloud")[0])
    time.sleep(1.0)
    after = c.image("tts-stopped")
    c.check("stopping removes the highlight", tinted(after) < 30, str(tinted(after)))
    c.check("and the player card", c.gone("button", "Next sentence", timeout=3))
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    c.click_node(c.wait_node("button", rx="^Voice settings", timeout=5))
    d = c.wait_node("dialog", "Preferences", timeout=8)
    c.check("Voice settings opens Preferences", d)
    c.check("on the Text to Speech page, at the engine row", c.wait_node(None, rx="^Voice engine", timeout=6))
    heads = c.find("label", "Voice")
    c.check("each group of the page is there once (one \"Voice\" heading)", len(heads) == 1, str(len(heads)))
    c.shot("tts-settings")
    c.check("the app is still alive", c.is_alive())


def _small(c, name):
    n = c.find(None, name)
    return bool(n) and n[0].box[2] <= 362 and n[0].box[3] <= 122


def bottom_pixels(img):
    """Pixels of the bottom band of the page, as a flat list (the card floats there)."""
    px = img.load()
    return [px[x, y][:3] for x in range(200, min(img.width, 1000), 6) for y in range(img.height - 260, img.height - 90, 6)]


@scenario
def tts_player_card_real(c):
    """The floating player card: appears over the page when reading starts, real clicks on its play/pause, next
    sentence and next chapter buttons, a click on the page does not dismiss it, the X stops reading."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "600"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    before = bottom_pixels(c.image("player-before"))
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens", play):
        return
    c.click_node(play)
    c.sb.key("Escape")
    nxt = c.wait_node("button", "Next sentence", timeout=6)
    if not c.check("the card appears with a next sentence button", nxt):
        return
    for name, role in (("Previous sentence", "button"), ("Close read aloud", "button")):
        c.check(f"the small player has {name}", c.find(role, name) or c.find(None, name))
    c.check("and it is small (at most 360 x 120)", _small(c, "Read aloud player"))
    c.click_node(c.wait_node("button", "Show the full player", timeout=4))
    time.sleep(0.4)
    for name, role in (("Next chapter", "button"), ("Previous chapter", "button"),
                       ("Position in the chapter", "slider"), ("Volume", "slider"),
                       ("Sleep timer", "toggle button"), ("Choose the voice", "toggle button")):
        c.check(f"the full player has {name}", c.find(role, name) or c.find(None, name))
    time.sleep(0.5)
    during = bottom_pixels(c.image("player-playing"))
    changed = sum(1 for a, b in zip(before, during) if sum(abs(x - y) for x, y in zip(a, b)) > 60)
    c.check("the card is painted over the bottom of the page", changed > len(before) * 0.08, f"{changed}/{len(before)}")
    c.click_node(nxt)
    time.sleep(0.8)
    c.check("next sentence keeps reading", c.find("button", "Pause reading"))
    c.click_node(c.find("button", "Pause reading")[0])
    c.check("pause shows resume", c.wait_node("button", "Resume reading", timeout=4))
    c.click_node(c.find("button", "Resume reading")[0])
    c.check("resume shows pause", c.wait_node("button", "Pause reading", timeout=4))
    c.click_node(c.find("button", "Next chapter")[0])
    time.sleep(1.2)
    c.check("next chapter keeps the card and the reading", c.find("button", "Pause reading"))
    c.shot("player-chapter-2")
    # a click on the page must not dismiss it
    c.sb.click(640, 300)
    time.sleep(0.5)
    c.check("a click on the page leaves the card", c.find("button", "Next sentence"))
    c.click_node(c.find("button", "Close read aloud")[0])
    c.check("the X stops reading and hides the card", c.gone("button", "Next sentence", timeout=4))
    c.check("the app is still alive", c.is_alive())


@scenario
def speed_reading_real(c):
    """Speed reading on the page: from the main menu; the card with real clicks and keys; the X closes it."""
    c.start()
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    c.click_node(c.wait_node("toggle button", rx="^Main menu"))
    item = c.wait_node(None, rx="^Speed Reading", timeout=5)
    if not c.check("the main menu has Speed Reading", item):
        return
    c.click_node(item)
    start = c.wait_node("button", "Start", timeout=6)
    if not c.check("the speed reading card appears paused with a Start button", start):
        return
    c.shot("rsvp-card")
    c.check("the card has Faster, Slower and the X", c.find("button", "Faster") and c.find("button", "Slower")
            and c.find("button", "Close speed reading"))
    c.click_node(start)
    c.check("a real click on Start runs it", c.wait_node("button", "Pause", timeout=4))
    time.sleep(1.0)
    c.sb.key("space")
    c.check("the Space key pauses it", c.wait_node("button", "Start", timeout=4))
    c.click_node(c.find("button", "Faster")[0])
    c.check("Faster is 325 words per minute", c.wait_node(None, rx="325 wpm", timeout=3))
    c.sb.key("Escape")
    c.check("Escape closes the card", c.gone("button", "Close speed reading", timeout=4))
    c.check("the app is still alive", c.is_alive())


def _work_log(c):
    """(t_ms, draws, layouts) lines the dev hook `work-watch` printed into the app log."""
    import os
    import re
    out = []
    with open(os.path.join(c.dir, "app.log"), errors="replace") as f:
        for line in f:
            m = re.search(r"work: t=(\d+) draws=(\d+) layouts=(\d+)", line)
            if m:
                out.append(tuple(int(g) for g in m.groups()))
    return out


def _drag_card(c, label, name, steps=60, step=(-5, -3)):
    """A real drag of a card across the window in small pointer steps. Returns the worst distance between
    where the card should be (it follows the pointer) and where it is, and the draws and layouts of the page
    canvas during the drag."""
    node = c.wait_node(None, name, timeout=6)
    if not c.check(f"{label}: the card is there", node):
        return None
    x0, y0, w, h = node.box
    px, py = 7 + x0 + w * 0.5, 37 + y0 + 14
    c.sb.move_to(px, py)
    c.sb.button(down=True)
    time.sleep(0.15)
    t_press = int(time.time() * 1000)
    worst, errs = 0.0, []
    for i in range(1, steps + 1):
        c.sb.move_by(*step)
        time.sleep(0.04)
        n = c.find(None, name)
        if not n:
            break
        bx, by = n[0].box[0], n[0].box[1]
        err = max(abs((bx - x0) - step[0] * i), abs((by - y0) - step[1] * i))
        errs.append(err)
        worst = max(worst, err)
    c.sb.button(down=False)
    t_release = int(time.time() * 1000)
    time.sleep(0.3)
    log = [r for r in _work_log(c) if t_press <= r[0] <= t_release]
    draws = log[-1][1] - log[0][1] if len(log) > 1 else -1
    layouts = log[-1][2] - log[0][2] if len(log) > 1 else -1
    print(f"METRIC {label}: worst error {worst:.1f}px over {len(errs)} steps, "
          f"mean {sum(errs) / max(len(errs), 1):.1f}px, page draws {draws}, page layouts {layouts}, "
          f"seconds {(t_release - t_press) / 1000:.1f}; first errors {[round(e) for e in errs[:6]]}, last {[round(e) for e in errs[-4:]]}")
    return worst, draws, layouts, len(errs)


@scenario
def cards_drag_real(c):
    """Dragging a card with a real pointer in 60 small steps: it stays under the pointer (within 3 px at the end of
    every step) and the page canvas is neither laid out again nor redrawn more than a few times."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "2500",
                       "BOOKX_DEV_SCRIPT": "wait:14000,reader:work-watch"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.5)
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens", play):
        return
    c.click_node(play)
    c.sb.key("Escape")
    time.sleep(0.8)
    r = _drag_card(c, "read aloud card", "Read aloud player")
    if r:
        c.check("the read aloud card tracks the pointer within 3 px at every step", r[0] <= 3.0, f"{r[0]:.1f}")
        c.check("and the page was not laid out again", r[2] == 0, str(r[2]))
        c.check("and was drawn only a few times (not once per pointer step)", 0 <= r[1] <= 12, str(r[1]))
    c.click_node(c.wait_node("toggle button", rx="^Main menu"))
    item = c.wait_node(None, rx="^Speed Reading", timeout=5)
    if not c.check("the main menu has Speed Reading", item):
        return
    c.click_node(item)
    time.sleep(0.8)
    r = _drag_card(c, "speed reading card", "Speed reading controls", step=(5, -2))
    if r:
        c.check("the speed reading card tracks the pointer within 3 px at every step", r[0] <= 3.0, f"{r[0]:.1f}")
        c.check("and the page was not laid out again", r[2] == 0, str(r[2]))
        c.check("and was drawn only a few times", 0 <= r[1] <= 12, str(r[1]))
    c.shot("cards-dragged")
    c.check("the app is still alive", c.is_alive())


@scenario
def voice_download_background_real(c):
    """Read Aloud with no voice: the dialog, the download starts, the user goes back to the library with a real
    click, the download is in the Background jobs list with progress, and it finishes there (toast gone,
    job Done) while the library is shown. The download is the app's mock (a few seconds, no network)."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "600", "BOOKX_TTS_MOCK_DOWNLOAD": "switch:9000"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens", play):
        return
    c.click_node(play)
    c.sb.key("Escape")
    go = c.wait_node("button", "Download", timeout=6)
    if not c.check("without a voice Read Aloud asks whether to download one", go):
        return
    c.shot("voice-offer")
    c.click_node(go)
    c.check("the card shows the voice being prepared",
            c.wait_node("label", rx="^Preparing voice", timeout=6))
    back = c.wait_node("button", "Back", timeout=5)
    if not c.check("the reader has a way back to the library", back):
        return
    c.click_node(back)
    c.check("the library is shown", c.wait_node("table cell", rx="Alice", timeout=8))
    jobs = c.wait_node("toggle button", rx="^Background jobs", timeout=8)
    if not c.check("the library has the Background jobs button", jobs):
        return
    c.click_node(jobs)
    row = c.wait_node("progress bar", rx="^Progress of Downloading", timeout=6)
    c.check("the voice download is listed with a progress bar while the library is shown", row)
    c.check("and a Cancel button", c.find("button", rx="^Cancel Downloading"))
    c.shot("voice-job")
    c.sb.key("Escape")
    time.sleep(10)
    c.click_node(c.wait_node("toggle button", rx="^Background jobs", timeout=5))
    done = c.wait_node(None, rx="Downloading the default voice", timeout=5)
    c.check("the job is still in the list after the download ended", done)
    c.check("and it is no longer running (no Cancel button)", not c.find("button", rx="^Cancel Downloading"))
    c.shot("voice-job-done")
    c.check("the app is still alive", c.is_alive())


def _group(c, name):
    n = c.find(None, name)
    return n[0] if n else None


def _inside(nodes, group):
    """The nodes whose centre lies inside the card `group` (both cards have buttons with the same names)."""
    gx, gy, gw, gh = group.box
    out = []
    for n in nodes:
        cx, cy = n.box[0] + n.box[2] / 2, n.box[1] + n.box[3] / 2
        if gx <= cx <= gx + gw and gy <= cy <= gy + gh:
            out.append(n)
    return out


def _real_drag(c, name, dx, dy, steps=24):
    g = _group(c, name)
    x, y, w, h = g.box
    px, py = 7 + x + w * 0.5, 37 + y + 14
    c.sb.move_to(px, py)
    c.sb.button(down=True)
    time.sleep(0.12)
    for _ in range(steps):
        c.sb.move_by(dx / steps, dy / steps)
        time.sleep(0.03)
    c.sb.button(down=False)
    time.sleep(0.4)
    return _group(c, name).box


def _cards_state(c):
    """What the two cards show: box, text of the sentence and its number, the word and the speed."""
    t, r = _group(c, "Read aloud player"), _group(c, "Speed reading controls")
    st = {"tts_box": t.box if t else None, "rsvp_box": r.box if r else None}
    if t:
        lab = _inside(c.find("label", "Sentence being read"), t)
        st["sentence"] = lab[0].get_text() if lab else None
        pos = _inside(c.find("label", rx=r"^\d+ / \d+$"), t)
        st["position"] = pos[0].name if pos else None
        st["tts_play"] = [n.name for n in _inside(c.find("button", rx="reading$|Read aloud from here"), t)]
    if r:
        wpm = _inside(c.find("label", rx=r"^\d+ wpm$"), r)
        st["wpm"] = wpm[0].name if wpm else None
        words = [n for n in _inside(c.find("label"), r)
                 if n.name and not n.name.endswith("wpm") and n.name != "Speed reading"]
        st["word"] = words[0].name if words else None
        st["rsvp_play"] = [n.name for n in _inside(c.find("button", rx="^(Start|Pause)$"), r)]
    return st


def _near(a, b, tol=2):
    return a is not None and b is not None and all(abs(p - q) <= tol for p, q in zip(a, b))


def _compare(c, label, before):
    now = _cards_state(c)
    c.check(f"{label}: the read aloud card is back", now["tts_box"] is not None)
    c.check(f"{label}: at the same place (within 2 px)", _near(now["tts_box"], before["tts_box"]),
            f"{now['tts_box']} vs {before['tts_box']}")
    c.check(f"{label}: showing the same sentence number of the chapter", now.get("position") == before["position"]
            and before["position"],
            f"{now.get('position')} vs {before['position']}")
    c.check(f"{label}: waiting, not playing (no autoplay)", now.get("tts_play") and
            not any(n.startswith("Pause") for n in now["tts_play"]), str(now.get("tts_play")))
    c.check(f"{label}: the speed reading card is back", now["rsvp_box"] is not None)
    c.check(f"{label}: at the same place (within 2 px)", _near(now["rsvp_box"], before["rsvp_box"]),
            f"{now['rsvp_box']} vs {before['rsvp_box']}")
    c.check(f"{label}: at the same word", now.get("word") == before["word"] and before["word"],
            f"{now.get('word')!r} vs {before['word']!r}")
    c.check(f"{label}: with the same speed", now.get("wpm") == before["wpm"], f"{now.get('wpm')} vs {before['wpm']}")
    c.check(f"{label}: waiting, not running", now.get("rsvp_play") == ["Start"], str(now.get("rsvp_play")))
    return now


def _leave_and_return(c):
    back = c.wait_node("button", "Back", timeout=5)
    c.click_node(back)
    c.check("the library is shown", c.wait_node("table cell", rx="Alice", timeout=8))
    time.sleep(1.0)
    c.check("the book opens again", c.open_book())
    time.sleep(2.5)


@scenario
def cards_restore_real(c):
    """Both cards, real drags and clicks: leave the book, come back, restart the app; the cards are where they
    were, show the same place, and do not play. A card closed with X stays closed."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "1500"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens", play):
        return
    c.click_node(play)
    c.sb.key("Escape")
    nxt = c.wait_node("button", "Next sentence", timeout=6)
    if not c.check("the read aloud card appears", nxt):
        return
    c.click_node(c.wait_node("button", "Show the full player", timeout=4))
    time.sleep(0.4)
    for _ in range(3):
        c.click_node(_inside(c.find("button", "Next sentence"), _group(c, "Read aloud player"))[0])
        time.sleep(0.4)
    c.click_node(_inside(c.find("button", rx="^Pause reading"), _group(c, "Read aloud player"))[0])
    time.sleep(0.4)
    first = _real_drag(c, "Read aloud player", -300, -150)
    c.check("the read aloud card was dragged somewhere else", first is not None)
    # speed reading
    c.click_node(c.wait_node("toggle button", rx="^Main menu"))
    c.click_node(c.wait_node(None, rx="^Speed Reading", timeout=5))
    c.check("the speed reading card appears", c.wait_node("button", "Start", timeout=6))
    time.sleep(0.5)
    second = _real_drag(c, "Speed reading controls", 220, -330)
    c.check("the speed reading card was dragged to another place", second is not None)
    r = _group(c, "Speed reading controls")
    c.click_node(_inside(c.find("button", "Start"), r)[0])
    time.sleep(1.6)
    c.click_node(_inside(c.find("button", "Pause"), r)[0])
    time.sleep(0.4)
    c.click_node(_inside(c.find("button", "Faster"), r)[0])
    time.sleep(0.5)
    before = _cards_state(c)
    c.check("both cards are on the page, apart", before["tts_box"] and before["rsvp_box"]
            and before["tts_box"][:2] != before["rsvp_box"][:2])
    c.check("read aloud shows a sentence number and the speed reading card a word",
            before["position"] and before["word"], str(before))
    c.shot("cards-before")
    page = c.page_label()
    _leave_and_return(c)
    c.shot("cards-after-return")
    _compare(c, "after leaving the book", before)
    c.check("the page shows the place of the card that was moved last (the speed-read word)",
            page is not None and c.page_label() is not None and abs(c.page_label()[0] - page[0]) <= 1,
            f"{c.page_label()} (before leaving: {page}; the word may sit on the other page of a spread)")
    # the same after the app was restarted
    c.restart_app()
    c.check("the book opens after the restart", c.open_book())
    time.sleep(2.5)
    c.shot("cards-after-restart")
    _compare(c, "after restarting the app", before)
    # close the speed reading card with its X: it does not come back
    c.click_node(c.wait_node("button", "Close speed reading", timeout=5))
    time.sleep(0.5)
    _leave_and_return(c)
    c.check("a card closed with X does not come back", c.gone(None, "Speed reading controls", timeout=3))
    c.check("the other one does", _group(c, "Read aloud player") is not None)
    c.restart_app()
    c.check("the book opens again after a restart", c.open_book())
    time.sleep(2.5)
    c.check("still no speed reading card after a restart", c.gone(None, "Speed reading controls", timeout=3))
    c.check("the read aloud card is still there", _group(c, "Read aloud player") is not None)
    c.check("the app is still alive", c.is_alive())


def _read_aloud_from_popover(c):
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    play = c.wait_node("button", "Read aloud from here", timeout=6)
    if not play:
        return False
    c.click_node(play)
    c.sb.key("Escape")
    return bool(c.wait_node("button", rx="^Pause reading", timeout=8))


def _no_failure_shown(c):
    return not c.find(None, rx=r"[Rr]eading aloud (failed|is not available)|only one|environment|try again")


@scenario
def tts_two_books_real(c):
    """The owner reads several books: read aloud in book A, back to the library, book B, back, book A again,
    then a rapid switch. Every Read Aloud starts, no error text appears, and each book keeps its own card
    (A's card comes back paused at A's sentence)."""
    c.start(books=("alice", "tiny"), extra_env={"BOOKX_TTS_MOCK": "700"})
    c.check("book A opens", c.open_book("Alice"))
    c.go_to_text()
    time.sleep(1.0)
    c.check("read aloud starts in book A", _read_aloud_from_popover(c))
    time.sleep(2.5)
    a_pos = _cards_text(c)
    c.click_node(c.wait_node("button", rx="^Pause reading", timeout=4))
    time.sleep(0.5)
    a_pos = _cards_text(c)
    _leave_to_library(c)
    c.check("book B opens", c.open_book("Masque"))
    time.sleep(1.5)
    c.check("read aloud starts in book B", _read_aloud_from_popover(c))
    c.check("no error is shown in book B", _no_failure_shown(c))
    time.sleep(1.5)
    _leave_to_library(c)
    c.check("book A opens again", c.open_book("Alice"))
    time.sleep(2.5)
    c.check("A's card is back, paused (no autoplay)", c.wait_node("button", rx="^(Resume reading|Read aloud from here)", timeout=6)
            and not c.find("button", rx="^Pause reading"))
    c.check("at A's sentence, not B's", _cards_text(c) == a_pos, f"{_cards_text(c)} vs {a_pos}")
    c.click_node(c.find("button", rx="^(Resume reading|Read aloud from here)")[0])
    c.check("playing again in book A", c.wait_node("button", rx="^Pause reading", timeout=8))
    c.check("no error is shown in book A", _no_failure_shown(c))
    # rapid switching: leave at once, another book, read, leave at once, back
    for title in ("Masque", "Alice", "Masque", "Alice"):
        _leave_to_library(c)
        c.check(f"{title} opens after a rapid switch", c.open_book(title))
        c.check(f"read aloud starts in {title} after a rapid switch", _read_aloud_from_popover(c))
        c.check(f"no error after the switch to {title}", _no_failure_shown(c))
    c.check("the app is still alive", c.is_alive())


def _cards_text(c):
    t = _group(c, "Read aloud player")
    if not t:
        return None
    pos = [n.name for n in _inside(c.find("label", rx=r"^\d+ / \d+$"), t)]
    return pos[0] if pos else tuple(n.box[:2] for n in [t])


def _leave_to_library(c):
    back = c.wait_node("button", "Back", timeout=5)
    c.click_node(back)
    c.wait_node("table cell", rx="Alice", timeout=8)
    time.sleep(0.8)


def _dark(img, box):
    """Dark pixels in a band of the screenshot: the icons and text of a bar, as against the bare page."""
    x0, y0, x1, y1 = box
    px = img.load()
    return sum(1 for x in range(x0, x1, 2) for y in range(y0, y1, 2) if sum(px[x, y][:3]) < 330)


@scenario
def reader_focus_real(c):
    """The reader focus (the bars away while reading) with the real pointer: while read aloud plays and the
    mouse is let go of, the header and the footer fade after about 1.5 s and only the page and the card are
    left; moving the pointer brings them back and they fade again; paused, they stay as before. Look at the
    screenshots focus-bars and focus-away."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "1500"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    head, foot = (0, 40, 1280, 78), (0, 742, 1280, 790)
    bars = c.image("focus-bars-start")
    bars_dark = _dark(bars, head) + _dark(bars, foot)
    c.check("the bars are there before reading (they have icons and text)", bars_dark > 150, str(bars_dark))
    c.check("read aloud starts", _read_aloud_from_popover(c))
    # closing the popover with Esc left the keyboard focus ring on the header button (which keeps the bars
    # by design); a click on the page takes the focus back to the page, as a mouse user would have it
    c.sb.click(640, 400)
    time.sleep(0.4)
    time.sleep(0.4)
    c.sb.move_to(660, 420)
    time.sleep(2.4)
    away = c.image("focus-away")
    away_dark = _dark(away, head) + _dark(away, foot)
    c.check("1.5 s after the mouse was let go the header and the footer are gone", away_dark < bars_dark * 0.3,
            f"{away_dark} vs {bars_dark}")
    c.check("the card is still there", c.find("button", "Next sentence"))
    c.sb.move_to(640, 20)  # the pointer goes to the top edge
    time.sleep(0.8)
    back = c.image("focus-bars-back")
    c.check("moving the pointer to the edge brings the bars back", _dark(back, head) + _dark(back, foot) > bars_dark * 0.6,
            f"{_dark(back, head) + _dark(back, foot)} vs {bars_dark}")
    c.sb.move_to(640, 400)
    time.sleep(2.6)
    again = c.image("focus-away-again")
    c.check("and when the pointer rests they go again", _dark(again, head) + _dark(again, foot) < bars_dark * 0.3)
    # paused: the normal behaviour, the bars stay for the old 4 s
    c.click_node(c.wait_node("button", rx="^Pause reading", timeout=4))
    c.sb.move_to(660, 420)
    time.sleep(2.4)
    paused = c.image("focus-paused")
    c.check("paused: after 2.4 s the bars are still there (the normal delay is 4 s)",
            _dark(paused, head) + _dark(paused, foot) > bars_dark * 0.6)
    time.sleep(3.0)
    later = c.image("focus-paused-later")
    c.check("and after 4 s they fade as before (the buttons go, the title and the status line stay)", _dark(later, head) + _dark(later, foot) < bars_dark * 0.8)
    c.check("the app is still alive", c.is_alive())


def _apart(a, b, gap=6):
    """Two boxes (x, y, w, h) that do not overlap, with room between them."""
    return (a is not None and b is not None and
            not (a[0] < b[0] + b[2] + gap and b[0] < a[0] + a[2] + gap and
                 a[1] < b[1] + b[3] + gap and b[1] < a[1] + a[3] + gap))


@scenario
def cards_no_overlap_real(c):
    """The layout manager with a real pointer: both cards open side by side and centred; the speed reading
    card is dragged onto the read aloud card and that one gives way (they never stack); after leaving the book
    and coming back they are still apart and the dragged one is where it was put. Look at the screenshots
    cards-side-by-side, cards-dragged-over and cards-restored."""
    c.start(extra_env={"BOOKX_TTS_MOCK": "1500"})
    c.check("the book opens", c.open_book())
    c.go_to_text()
    time.sleep(1.0)
    c.click_node(c.wait_node("toggle button", rx="^Read aloud"))
    play = c.wait_node("button", "Read aloud from here", timeout=5)
    if not c.check("the popover opens", play):
        return
    c.click_node(play)
    c.sb.key("Escape")
    if not c.check("the read aloud card appears", c.wait_node("button", "Next sentence", timeout=6)):
        return
    c.click_node(c.wait_node("toggle button", rx="^Main menu"))
    c.click_node(c.wait_node(None, rx="^Speed Reading", timeout=5))
    if not c.check("the speed reading card appears", c.wait_node("button", "Start", timeout=6)):
        return
    time.sleep(1.2)  # the first card slides aside
    st = _cards_state(c)
    t, r = st["tts_box"], st["rsvp_box"]
    c.shot("cards-side-by-side")
    c.check("both cards are there", t is not None and r is not None)
    if t is None or r is None:
        return
    c.check("they do not overlap", _apart(t, r), f"{t} vs {r}")
    left, right = sorted([t, r])
    gap = right[0] - (left[0] + left[2])
    c.check("side by side with a gap of about 12 px", 8 <= gap <= 16, str(gap))
    centre = (left[0] + right[0] + right[2]) / 2
    c.check("centred as a group in the window", abs(centre - 640) <= 12, f"{centre}")
    c.check("on the same line at the bottom", abs((t[1] + t[3]) - (r[1] + r[3])) <= 3 and t[1] > 450,
            f"{t} {r}")
    # the speed reading card is dragged onto the read aloud card
    tx, ty = t[0] + t[2] / 2, t[1] + t[3] / 2
    rx_, ry = r[0] + r[2] / 2, r[1] + r[3] / 2
    moved = _real_drag(c, "Speed reading controls", tx - rx_, ty - 40 - ry)
    time.sleep(0.8)
    st = _cards_state(c)
    c.shot("cards-dragged-over")
    c.check("the dragged card went where it was taken", _near(st["rsvp_box"], moved, 3),
            f"{st['rsvp_box']} vs {moved}")
    c.check("dropped on the other card, they still do not overlap", _apart(st["tts_box"], st["rsvp_box"]),
            f"{st['tts_box']} vs {st['rsvp_box']}")
    c.check("the read aloud card gave way", not _near(st["tts_box"], t, 4), f"{st['tts_box']} vs {t}")
    # and the other way round: the read aloud card is dragged over the speed reading card
    over = _real_drag(c, "Read aloud player", 0, -( st["tts_box"][1] - st["rsvp_box"][1]) - 20)
    time.sleep(0.8)
    st2 = _cards_state(c)
    c.check("dragging the read aloud card over the other: no overlap", _apart(st2["tts_box"], st2["rsvp_box"]),
            f"{st2['tts_box']} vs {st2['rsvp_box']}")
    c.check("and the dragged one is under the pointer", _near(st2["tts_box"], over, 3), f"{st2['tts_box']} vs {over}")
    _leave_and_return(c)
    st3 = _cards_state(c)
    c.shot("cards-restored")
    c.check("after coming back both cards are there", st3["tts_box"] is not None and st3["rsvp_box"] is not None)
    c.check("and do not overlap", _apart(st3["tts_box"], st3["rsvp_box"]), f"{st3['tts_box']} vs {st3['rsvp_box']}")
    c.check("the card moved last is where the user put it", _near(st3["tts_box"], st2["tts_box"], 3),
            f"{st3['tts_box']} vs {st2['tts_box']}")
    c.check("the other is where it was pushed to (the wishes are recomputed to the same places)",
            _near(st3["rsvp_box"], st2["rsvp_box"], 3), f"{st3['rsvp_box']} vs {st2['rsvp_box']}")
    c.check("the app is still alive", c.is_alive())
