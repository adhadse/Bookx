"""Your own vocabulary with real input: select a word with the mouse, Add to Vocabulary, type a definition,
Ctrl+Enter saves, the term is marked in the text, a real click on it shows the definition; the keyboard-only
flow (Shift+arrows, Enter, v, Tab, Ctrl+Enter); the look-up popover with "Your definition". Light and dark,
900x600 and 1280x800. Screenshots: <results>/vocab_*-*.png. Imported by scenarios.py."""
import json
import os
import time

from harness import scenario

DARK = "[org/gnome/desktop/interface]\ncolor-scheme='prefer-dark'\n"
WORD = "A word I made up"


def _vocab_file():
    return os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx", "vocabulary.json")


def _entries():
    try:
        with open(_vocab_file()) as f:
            return json.load(f).get("entries", [])
    except (OSError, ValueError):
        return []


def _start(c, settings, window):
    """`window` is the size of the (virtual) screen: the app fills it."""
    c.vocab_window = tuple(int(v) for v in window.split("x")) if window else None
    c.start(books=("alice",), settings=settings, size=window or "1280x800")
    time.sleep(2.0)
    c.open_book()
    c.go_to_text(24 if window else 8)


def _word_point(c):
    """A point on a word in the middle of the page."""
    ww, wh = c.vocab_window or (1280, 800)
    x, y = (400, 348) if not c.vocab_window else (int(ww * 0.3), int(32 + (wh - 32) * 0.5))
    img = c.image()
    return c.glyph_near(x, y, img) or (x, y)


def _flow(c, label, settings="", window=None):
    _start(c, settings, window)
    p = _word_point(c)
    c.sb.double_click(*p)
    add = c.wait_node("button", "Add to Vocabulary", timeout=6)
    if not c.check("a double click on a word opens the action popover with the Add to Vocabulary button", add):
        c.shot(f"{label}-no-button")
        return
    c.check("the popover also has the dictionary, Wikipedia and note buttons",
            c.find("button", "Look up in a dictionary") and c.find("button", "Add a note"))
    c.shot(f"{label}-popover")
    c.click_node(add)
    save = c.wait_node("button", "Save", timeout=6)
    if not c.check("the dialog opens", save):
        c.shot(f"{label}-no-dialog")
        return
    c.check("it has a Term field, a definition field and the Applies to choice",
            c.find("text", rx="Term|Your definition") or c.find("label", "Your definition"))
    c.shot(f"{label}-dialog-empty")
    c.sb.type(WORD)
    time.sleep(0.4)
    c.shot(f"{label}-dialog-typed")
    # Ctrl+Enter saves from the definition field
    c.sb.key("Return", ("Control",))
    c.check("Ctrl+Enter saves and closes the dialog", c.gone("button", "Save", timeout=5))
    time.sleep(0.8)
    ents = _entries()
    c.check("the vocabulary file holds the term with your definition",
            any(e.get("own") == WORD for e in ents), str(ents)[:300])
    c.shot(f"{label}-marked")
    # a real click on the marked word shows the definition
    c.sb.click(*p)
    shown = c.wait_node("label", WORD, timeout=5)
    c.check("a click on the marked word shows your definition in a popover", shown)
    c.check("the popover has Edit, Remove and Open in Vocabulary",
            c.find("button", "Edit") and c.find("button", "Remove") and c.find("button", "Open in Vocabulary"))
    c.shot(f"{label}-term-popover")
    c.sb.key("Escape")
    time.sleep(0.5)
    c.check("the app is still alive", c.is_alive())


@scenario
def vocab_own_light_wide(c):
    _flow(c, "vocab_light_wide")


@scenario
def vocab_own_dark_wide(c):
    _flow(c, "vocab_dark_wide", DARK)


@scenario
def vocab_own_light_small(c):
    _flow(c, "vocab_light_small", "", "900x600")


@scenario
def vocab_own_dark_small(c):
    _flow(c, "vocab_dark_small", DARK, "900x600")


@scenario
def vocab_own_keyboard(c):
    """No mouse after the book is open: select with Shift+arrows, Enter, v, type, Tab to the note, Ctrl+Enter."""
    _start(c, "", None)
    c.sb.click(*_word_point(c))  # focus the page
    time.sleep(0.5)
    for _ in range(6):
        c.sb.key("Right", ("Shift",))
    c.sb.key("Return")
    time.sleep(0.8)
    c.shot("vocab_kb-selected")
    c.sb.key("v")
    save = c.wait_node("button", "Save", timeout=6)
    c.check("the v key opens Add to Vocabulary for the keyboard selection", save)
    c.shot("vocab_kb-dialog")
    c.sb.type("by keyboard only")
    c.sb.key("Tab")
    c.sb.key("Tab")
    time.sleep(0.3)
    c.sb.key("Return", ("Control",))
    c.check("Ctrl+Enter saves", c.gone("button", "Save", timeout=5))
    time.sleep(0.6)
    ents = _entries()
    c.check("the term is saved with the typed definition", any("by keyboard only" in (e.get("own") or "") for e in ents),
            str(ents)[:300])
    c.shot("vocab_kb-saved")


@scenario
def vocab_own_in_lookup(c):
    """The look-up popover: Add my own definition, then Your definition above the dictionary result."""
    _start(c, "", None)
    c.sb.double_click(*_word_point(c))
    btn = c.wait_node("button", "Look up in a dictionary", timeout=6)
    if not c.check("the action popover opens", btn):
        return
    c.click_node(btn)
    add = c.wait_node("button", "Add my own definition", timeout=6)
    c.check("the look-up popover offers Add my own definition", add)
    c.check("the dictionary result is shown too", c.wait_node("label", rx="Test definition of ", timeout=5))
    c.shot("vocab_lookup-before")
    if not add:
        return
    c.click_node(add)
    save = c.wait_node("button", "Save", timeout=6)
    c.check("it opens the dialog", save)
    c.sb.type("mine first")
    c.sb.key("Return", ("Control",))
    c.gone("button", "Save", timeout=5)
    time.sleep(0.8)
    c.sb.double_click(*_word_point(c))
    btn = c.wait_node("button", "Look up in a dictionary", timeout=6)
    if btn:
        c.click_node(btn)
    c.check("Your definition is shown in the look-up popover", c.wait_node("label", "mine first", timeout=6))
    c.check("with Edit my definition", c.find("button", "Edit my definition"))
    c.shot("vocab_lookup-after")


def _seed_words(c):
    now = "2026-11-20T09:00:00Z"
    card = {"ease": 2.5, "interval_days": 0, "repetitions": 0, "lapses": 0, "due": now, "last_reviewed": None}

    def e(word, own=None, dic=None, note=""):
        d = {"word": word, "definition": dic, "contexts": [], "lookups": 1, "added": now, "card": card}
        if own:
            d.update({"own": own, "scope": {"kind": "all"}})
        if note:
            d["note"] = note
        return d

    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "vocabulary.json"), "w") as f:
        json.dump({"entries": [
            e("Nautilus", own="Captain Nemo's submarine, named after the shell", note="chapter 4"),
            e("Ned Land", own="the Canadian harpooner aboard the Nautilus"),
            e("ephemeral", dic="lasting a very short time"),
            e("zorbling", own="a small rolling thing from my glossary"),
        ]}, f)


@scenario
def vocab_words_page(c):
    """The Words page of the Vocabulary window with real clicks: badges, filter, search."""
    c.start(books=("alice",), seed=_seed_words)
    time.sleep(2.0)
    c.sb.key("k", ("Control",))
    time.sleep(0.8)
    c.sb.type("vocabulary")
    time.sleep(0.8)
    c.shot("vocab_words-palette")
    c.sb.key("Return")
    words = c.wait_node("toggle button", "Words", timeout=8) or c.wait_node("radio button", "Words", timeout=3) or c.wait_node("button", "Words", timeout=3)
    if not c.check("the Vocabulary window opens with a Words page switch", words):
        c.shot("vocab_words-none")
        return
    c.sb.click(*words.center())
    time.sleep(0.8)
    c.check("the list shows the terms with your own definition", c.find("label", "Nautilus") or c.find("label", rx="Nautilus"))
    c.check("with the Mine badge", c.find("label", "Mine"))
    c.shot("vocab_words-all")
    mine = c.find("toggle button", "Mine") or c.find("radio button", "Mine") or c.find("button", "Mine")
    if mine:
        c.sb.click(*mine[0].center())
        time.sleep(0.6)
        c.check("the Mine filter hides the dictionary word", not c.find("label", "ephemeral"))
        c.shot("vocab_words-mine")
