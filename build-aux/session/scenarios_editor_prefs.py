"""Editor shortcuts and snippets with real input: a rebound editor key works and the old one does not, Tab
expands snippets (also around selected text). Imported by scenarios.py. See docs/calibre-editor-gap-analysis.md
CE-383, CE-403."""
import json
import os
import time

from harness import scenario
from scenarios_editor_tools import open_editor, status_line


def find_bar_visible(c):
    return any(n.showing for n in c.find("button", "Find All"))


@scenario
def editor_rebind_real_keys(c):
    """The editor's shortcuts come from the registry: Ctrl+F is moved to Ctrl+Alt+F and the real keys follow."""
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "shortcuts.json"), "w") as f:
        json.dump({"version": 1, "bindings": {"editor.find": ["<Control><Alt>f"], "editor.go-to-line": ["F4"], "editor.replace-bar": []}}, f)
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("f", ("Control",))
    time.sleep(0.8)
    c.check("the old key (Ctrl+F) no longer opens the find bar", not find_bar_visible(c))
    c.sb.key("h", ("Control",))
    time.sleep(0.8)
    c.check("the second default of the same action (Ctrl+H) does not either", not find_bar_visible(c))
    c.sb.key("f", ("Control", "Alt"))
    ok = c.wait(lambda: find_bar_visible(c), 6)
    c.check("the new key (Ctrl+Alt+F) opens the find bar", ok)
    c.shot("rebound-find")
    c.sb.key("Escape")
    time.sleep(0.5)
    c.click_node(code)
    c.sb.key("F4")
    time.sleep(0.6)
    c.sb.type("5")
    c.shot("rebound-goto")
    c.sb.key("Return")
    now = c.wait(lambda: (status_line(c) or ("", 0))[1] == 5, 6)
    c.check("F4, the new key of Go to Line, opens it: typing 5 and Enter goes to line 5", now, str(status_line(c)))
    c.sb.key("s", ("Control",))
    time.sleep(0.6)
    c.check("a key that was not changed still works (Ctrl+S) and the app lives", c.is_alive())


@scenario
def editor_snippet_tab(c):
    """Tab expands a snippet trigger with real keys; with text selected after the trigger the text is wrapped."""
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type("\n")
    c.sb.type("Lorem")
    time.sleep(0.5)
    c.sb.key("Tab")
    time.sleep(0.8)
    text = code.get_text()
    c.check("Lorem and Tab insert filler paragraphs", text.count("Lorem ipsum dolor sit amet") >= 1 and "<p>Lorem ipsum" in text,
            text[-120:])
    c.check("the trigger word itself is gone", "\nLoremLorem" not in text and not text.rstrip().endswith("Loremlorem"))
    c.shot("lorem")
    c.sb.key("End", ("Control",))
    c.sb.type("\n<a")
    time.sleep(0.3)
    c.sb.type("wrapped")
    time.sleep(0.4)
    c.note(f"after typing: {code.get_text()[-40:]!r}")
    for _ in range(len("wrapped")):
        c.sb.key("Left", ("Shift",))
    time.sleep(0.3)
    c.note(f"selected: {code.get_text()[-40:]!r}")
    c.sb.key("Tab")
    time.sleep(0.8)
    text = code.get_text()
    c.check("<a, selected text and Tab wrap the selection in a link", '<a href="#">wrapped</a>' in text, repr(text[-100:]))
    c.shot("wrap-link")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_prefs_page(c):
    """Editor Preferences from the command palette: the page shows the settings, a change is applied at once."""
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    code = max(c.find("text"), key=lambda n: n.box[2] * n.box[3])
    c.click_node(code)
    c.sb.key("p", ("Control", "Shift"))
    time.sleep(0.8)
    c.sb.type("Editor Preferences")
    time.sleep(0.6)
    c.sb.key("Return")
    dlg = c.wait(lambda: [n for n in c.find(None, "Preferences") if n.showing], 8)
    if not c.check("the palette opens Preferences on the Editor page", dlg):
        c.shot("no-prefs")
        return
    time.sleep(0.8)
    c.shot("editor-prefs")
    c.check("the app is still alive", c.is_alive())


@scenario
def dialog_close_keys(c):
    """X9 / 5.7.5: Ctrl+W and Esc close a dialog under real input (Preferences, then Keyboard Shortcuts' neighbour: Sync)."""
    c.start()
    for key, mods, what in (("w", ("Control",), "Ctrl+W"), ("Escape", (), "Esc")):
        c.sb.key(",", ("Control",))
        dlg = c.wait(lambda: [n for n in c.find(None, "Preferences") if n.showing], 8)
        if not c.check(f"Ctrl+, opens Preferences before {what}", dlg):
            c.shot("no-prefs")
            return
        time.sleep(0.6)
        c.sb.key(key, mods)
        gone = c.wait(lambda: not [n for n in c.find(None, "Preferences") if n.showing], 6)
        c.check(f"{what} closes the Preferences dialog", gone)
    c.check("the app is still alive", c.is_alive())
