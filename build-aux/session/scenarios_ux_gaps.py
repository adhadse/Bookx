"""Library and reader UX gaps with real input: rebound keyboard shortcuts. Imported by scenarios.py."""
import json
import os
import time

from harness import scenario


@scenario
def shortcuts_real_keys(c):
    """A shortcut set in the file works with the real keyboard; the key it replaced no longer does."""
    cfg = os.path.join(os.environ["XDG_CONFIG_HOME"], "bookx")
    os.makedirs(cfg, exist_ok=True)
    with open(os.path.join(cfg, "shortcuts.json"), "w") as f:
        json.dump({"version": 1, "bindings": {"reader.next-page": ["j"], "app.palette": ["<Control>j"]}}, f)
    c.start()
    # library: the palette follows its new key, the old one does nothing
    c.sb.key("k", ("Control",))
    time.sleep(0.8)
    c.check("the old palette key (Ctrl+K) no longer opens the palette", not c.find(None, "Command palette search"))
    c.sb.key("j", ("Control",))
    entry = c.wait_node(None, "Command palette search", timeout=6)
    c.check("the new key (Ctrl+J) opens the palette", entry)
    c.sb.key("Escape")
    c.check("Escape closes it", c.gone(None, "Command palette search", timeout=4))
    c.open_book()
    time.sleep(1.5)
    p0 = c.page_label()
    c.check("the reader shows page numbers", p0)
    c.sb.key("Right")
    time.sleep(0.8)
    p1 = c.page_label()
    c.check("the default next-page key (Right) does nothing now", p1 == p0, f"{p0} -> {p1}")
    c.sb.key("j")
    time.sleep(1.0)
    p2 = c.page_label()
    c.check("the new key (j) turns the page", p0 and p2 and p2[0] > p0[0], f"{p0} -> {p2}")
    c.sb.key("Left")
    time.sleep(0.8)
    p3 = c.page_label()
    c.check("keys that were not changed still work (Left goes back)", p2 and p3 and p3[0] < p2[0], f"{p2} -> {p3}")
    c.shot("rebound")


@scenario
def reader_goto_keys(c):
    """Ctrl+G opens Go to…, a percent and Enter jump there, Alt+Left comes back and Alt+Right goes forward again."""
    c.start()
    c.open_book()
    c.go_to_text()
    p0 = c.page_label()
    c.check("the reader shows page numbers", p0)
    c.sb.key("g", ("Control",))
    entry = c.wait_node(None, "Go to page, percent, location or chapter", timeout=6)
    if not c.check("Ctrl+G opens the Go to dialog with the field focused", entry):
        c.shot("no-goto")
        return
    c.sb.type("90%")
    time.sleep(0.5)
    c.shot("goto-dialog")
    c.sb.key("Return")
    time.sleep(1.5)
    p1 = c.page_label()
    c.check("Enter jumps to 90% of the book", p0 and p1 and p1[0] > p0[0] + 10, f"{p0} -> {p1}")
    c.sb.key("Left", ("Alt",))
    time.sleep(1.2)
    p2 = c.page_label()
    c.check("Alt+Left goes back to where you were", p2 and p0 and abs(p2[0] - p0[0]) <= 2, f"{p0} -> {p1} -> {p2}")
    c.sb.key("Right", ("Alt",))
    p3 = c.wait(lambda: c.page_label() if c.page_label() and c.page_label()[0] > p0[0] + 5 else None, 6)
    c.check("Alt+Right goes forward again", p3 and p1 and abs(p3[0] - p1[0]) <= 2, f"{p3} vs {p1}")
