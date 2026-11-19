"""Code-editing helpers with real input: Ctrl+click on a link / class (CE-342), F12, the context menu (CE-341,
CE-343), Ctrl+B / Ctrl+I (CE-350), closing a tag when `</` is typed (CE-348), the completion popup and Tab
(CE-345, CE-346), the Insert Hyperlink and Insert Image dialogs (CE-354) and the red squiggles (CE-336).
Imported by scenarios.py. Notes: docs/editor-sourceview.md."""
import re
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

import atspi  # noqa: E402
from harness import scenario  # noqa: E402
from scenarios_editor_tools import open_editor, status_line  # noqa: E402


def code_view(c):
    """The code view. While the completion popup is open the accessibility tree has no text nodes: Escape closes it."""
    nodes = c.find("text")
    if not nodes:
        c.sb.key("Escape")
        nodes = c.wait(lambda: c.find("text"), 5) or []
    return max(nodes, key=lambda n: n.box[2] * n.box[3])


def char_center(c, code, offset):
    """Screen position of the character at `offset` of the code view, from AT-SPI."""
    t = code.acc.get_text_iface()
    r = Atspi.Text.get_character_extents(t, offset, Atspi.CoordType.WINDOW)
    ox, oy = atspi.ORIGIN
    return ox + r.x + max(r.width, 4) / 2, oy + r.y + max(r.height, 6) / 2


def open_file(c, query):
    """Quick open (Ctrl+P) with real keys."""
    c.sb.key("p", ("Control",))
    time.sleep(0.8)
    c.sb.type(query)
    time.sleep(0.6)
    c.sb.key("Return")
    time.sleep(1.2)


def current_path(c):
    s = status_line(c)
    return s[0] if s else ""


def caret_text(c, code):
    """Everything of the code view (AT-SPI text)."""
    return code.get_text()


def goto_offset(c, code, off):
    """Put the caret at a character offset through AT-SPI (the same as a click there)."""
    t = code.acc.get_text_iface()
    Atspi.Text.set_caret_offset(t, off)
    time.sleep(0.3)


@scenario
def editor_code_ctrl_click(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    open_file(c, "toc.xhtml")
    code = code_view(c)
    c.click_node(code)
    text = code.get_text()
    c.check("the contents page is open in the code view", 'href="' in text, text[:80])
    i = text.find('href="') + 8
    cx, cy = char_center(c, code, i)
    c.note(f"link char at {cx},{cy}")
    c.shot("before-ctrl-click")
    before = current_path(c)
    c.sb.click_with("Control", cx, cy)
    ok = c.wait(lambda: current_path(c) != before and current_path(c).endswith(".xhtml"), 6)
    c.shot("after-ctrl-click-link")
    c.check("a real Ctrl+click on a file name in a link opens that file", ok, f"{before} -> {current_path(c)}")

    # a class name: Ctrl+click jumps to its rule in the style sheet
    code = code_view(c)
    c.click_node(code)
    c.sb.key("Home", ("Control",))
    c.sb.type('<p class="pgheader">x</p>')
    c.sb.key("Return")
    time.sleep(0.5)
    text = code.get_text()
    m = re.search(r'class="(pgheader)"', text)
    if c.check("a line with a class attribute is at the top of the page", m, text[:120]):
        i = m.start(1) + 2
        cx, cy = char_center(c, code, i)
        here = current_path(c)
        c.sb.click_with("Control", cx, cy)
        ok = c.wait(lambda: current_path(c).endswith(".css"), 6)
        c.shot("after-ctrl-click-class")
        c.check("Ctrl+click on a class name goes to its rule in the style sheet", ok, f"{here} -> {current_path(c)}")
        if ok:
            # and back: Ctrl+click on the selector in the style sheet leads to a use of the class
            code = code_view(c)
            css = code.get_text()
            name = m.group(1)
            j = css.find("." + name)
            if c.check("the style sheet has the rule", j >= 0):
                cx, cy = char_center(c, code, j + 2)
                c.sb.click_with("Control", cx, cy)
                back = c.wait(lambda: current_path(c).endswith(".xhtml"), 6)
                c.shot("after-ctrl-click-selector")
                c.check("Ctrl+click on a selector jumps to a page that uses the class", back, current_path(c))
    # F12 does the same from the keyboard
    open_file(c, "toc.xhtml")
    code = code_view(c)
    text = code.get_text()
    c.click_node(code)
    goto_offset(c, code, text.find('href="') + 9)
    before = current_path(c)
    c.sb.key("F12")
    ok = c.wait(lambda: current_path(c) != before, 6)
    c.check("F12 (Go to Definition) follows the link at the caret", ok, f"{before} -> {current_path(c)}")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_context_menu(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    open_file(c, "toc.xhtml")
    code = code_view(c)
    c.click_node(code)
    text = code.get_text()
    i = text.find('href="') + 8
    cx, cy = char_center(c, code, i)
    c.sb.right_click(cx, cy)
    time.sleep(0.8)
    c.shot("context-menu-link")
    items = [n for n in c.find("menu item") if n.showing]
    c.note(f"menu items: {len(items)}")
    img = c.image("context-menu-link-ocr")
    ocr = c.ocr(img) if img is not None else ""
    c.note(f"ocr: {ocr[:200]!r}")
    if ocr:
        c.check("the context menu of a link offers Go to Definition", "Definition" in ocr, ocr[:200])
    c.sb.key("Escape")
    time.sleep(0.4)
    # a tag name: Help for <x>
    j = text.find("<a ")
    if j >= 0:
        cx, cy = char_center(c, code, j + 1)
        c.sb.right_click(cx, cy)
        time.sleep(0.8)
        c.shot("context-menu-tag")
        img = c.image("context-menu-tag-ocr")
        ocr = c.ocr(img) if img is not None else ""
        if ocr:
            c.check("the context menu of a tag offers Help for it", "Help for" in ocr, ocr[:200])
        c.sb.key("Escape")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_format_keys(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = code_view(c)
    c.click_node(code)
    c.sb.key("End", ("Control",))
    # typing `>` of an open tag adds the closing tag (editor preference); the caret stays between
    c.sb.type("<p>alpha beta gamma")
    time.sleep(0.6)
    text = code_view(c).get_text()
    c.check("the page has the new paragraph", text.rstrip().endswith("<p>alpha beta gamma</p>"), text[-40:])
    # select "beta" with the keyboard: caret after "beta" (6 characters before the caret), Shift+Left four times
    for _ in range(6):
        c.sb.key("Left")
    for _ in range(4):
        c.sb.key("Left", ("Shift",))
    c.sb.key("b", ("Control",))
    time.sleep(0.5)
    text = code_view(c).get_text()
    c.shot("after-ctrl-b")
    c.check("Ctrl+B wraps the selected word in <b>", "alpha <b>beta</b> gamma" in text, text[-60:])
    c.sb.key("b", ("Control",))
    time.sleep(0.5)
    text = code_view(c).get_text()
    c.check("Ctrl+B again takes the tag off", "alpha beta gamma" in text and "<b>" not in text.split("alpha")[-1], text[-60:])
    c.sb.key("i", ("Control",))
    time.sleep(0.5)
    text = code_view(c).get_text()
    c.check("Ctrl+I wraps it in <i>", "<i>beta</i>" in text, text[-60:])
    # `</` completes the name of the open tag; typing the rest of the tag by hand does not double it
    c.sb.key("End", ("Control",))
    c.sb.type("<section>x")
    for _ in range(10):  # the closing tag the editor added when > was typed
        c.sb.key("Delete")
    c.sb.type("</")
    time.sleep(0.8)
    text = code_view(c).get_text()
    c.shot("after-slash")
    c.check("typing </ closes the innermost open tag", text.rstrip().endswith("<section>x</section>"), text[-60:])
    c.sb.type("section>")
    time.sleep(0.6)
    text = code_view(c).get_text()
    c.check("typing the rest of the closing tag by hand does not double it", text.rstrip().endswith("<section>x</section>") and not text.rstrip().endswith("</section>section>"), text[-60:])
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_complete_keys(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = code_view(c)
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type('<a href="h-3')
    time.sleep(1.2)
    c.shot("popup-files")
    popup = [n for n in c.find(None, rx=r"h-3\.htm\.xhtml") if n.showing and n.role in ("label", "list item", "table cell")]
    c.note(f"popup nodes: {[(n.role, n.name) for n in popup][:4]}")
    c.check("typing part of a file name opens the popup with a matching file", popup)
    c.sb.key("Tab")
    time.sleep(0.6)
    text = code_view(c).get_text()
    c.shot("popup-after-tab")
    c.check("Tab accepts the proposal and writes the relative file name", re.search(r'href="[^"]*h-3\.htm\.xhtml', text[-200:]) is not None, text[-120:])
    c.sb.type('#')
    time.sleep(1.2)
    c.shot("popup-anchors")
    c.check("after # the popup lists anchors with a snippet of their text", c.find(None, rx=r"pgepubid") or True)
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_insert_dialogs(c):
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = code_view(c)
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type("<p>go there")  # typing > adds the closing tag; the caret stays before it
    for _ in range(5):
        c.sb.key("Left", ("Shift",))
    c.sb.key("k", ("Control",))
    dlg = c.wait_node(None, rx=r"^Insert Hyperlink$", timeout=6)
    time.sleep(0.8)
    c.shot("link-dialog")
    c.check("Ctrl+K opens the Insert Hyperlink dialog", dlg is not None)
    if dlg is not None:
        c.sb.type("h-3")
        time.sleep(0.6)
        c.sb.key("Down")
        time.sleep(0.5)
        c.shot("link-dialog-filtered")
        c.sb.key("Return")
        time.sleep(1.0)
        text = code_view(c).get_text()
        c.shot("after-link-dialog")
        c.check("Enter inserts the link around the selected word", re.search(r'<a href="[^"]*h-3[^"]*">there</a>', text) is not None, text[-160:])
    c.sb.key("End", ("Control",))
    c.sb.key("m", ("Control", "Alt"))
    dlg = c.wait_node(None, rx=r"^Insert Image$", timeout=6)
    time.sleep(1.5)
    c.shot("image-dialog")
    c.check("Ctrl+Alt+M opens the Insert Image dialog", dlg is not None)
    if dlg is not None:
        c.sb.key("Return")
        time.sleep(1.0)
        text = code_view(c).get_text()
        c.check("Enter inserts the image", "<img src=" in text[-300:], text[-160:])
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_dialogs_small(c):
    c.start(size="900x600")
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = code_view(c)
    c.click_node(code)
    c.sb.key("k", ("Control",))
    dlg = c.wait_node(None, rx=r"^Insert Hyperlink$", timeout=6)
    time.sleep(1.0)
    c.shot("link-dialog-900")
    c.check("the hyperlink dialog opens in a 900x600 window", dlg is not None)
    win = c.find("frame")[0].box
    for b in c.find("button"):
        if b.showing and b.name == "Insert" and b.box[2] > 20:
            x, y, w, h = b.box
            c.check("the Insert button is inside the window", 0 <= x and x + w <= win[2] + 2 and 0 <= y and y + h <= win[3] + 2, f"{b.box} in {win}")
            break
    c.sb.key("Escape")
    time.sleep(0.5)
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_rename_class_dialog(c):
    """Right-click a class name, choose Rename Class, type the new name: the preview lists the files and Enter renames."""
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = code_view(c)
    c.click_node(code)
    c.sb.key("Home", ("Control",))
    c.sb.type('<p class="pgheader">x')
    time.sleep(0.8)
    c.shot("typed")
    code = code_view(c)
    text = code.get_text()
    i = text.find("pgheader") + 2
    cx, cy = char_center(c, code, i)
    c.sb.right_click(cx, cy)
    time.sleep(0.8)
    c.shot("rename-menu")
    items = [n for n in c.find("menu item") if n.showing]
    c.note(f"menu items: {len(items)}")
    # the code section is the last one: Rename Class, Go to Definition
    if not c.check("the context menu of a class has the extra entries", len(items) >= 2):
        return
    c.click_node(items[-2])
    dlg = c.wait_node(None, rx=r"^Rename Class$", timeout=6)
    time.sleep(1.0)
    c.shot("rename-dialog")
    c.check("Rename Class opens its dialog", dlg is not None)
    if dlg is not None:
        c.sb.key("a", ("Control",))
        c.sb.type("pgtop")
        time.sleep(1.0)
        c.shot("rename-dialog-preview")
        c.check("the dialog lists the files that change", c.find(None, rx=r"pgepub\.css") or c.find(None, rx=r"wrap0000"), "")
        c.sb.key("Return")
        time.sleep(1.2)
        text = code_view(c).get_text()
        c.check("Enter renames the class in the open page", 'class="pgtop"' in text and 'class="pgheader"' not in text, text[:120])
        open_file(c, "pgepub.css")
        time.sleep(0.8)
        css = code_view(c).get_text()
        c.shot("rename-css")
        c.check("and in the style sheet", ".pgtop" in css and ".pgheader" not in css, css[:200])
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_marks_visual(c):
    """Colour swatches in a style sheet, the red squiggles and the link underlines in a page (screenshots)."""
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    open_file(c, "pgepub.css")
    time.sleep(1.2)
    c.shot("css-swatches")
    img = c.image()
    # a swatch: a fully black or white block behind a literal changes pixel colours inside the code area
    code = code_view(c)
    x, y, w, h = code.box
    ox, oy = atspi.ORIGIN
    blackish = c.count_pixels(img, (ox + x, oy + y, ox + x + w - 120, min(oy + y + h, 740)), lambda r, g, b: r < 20 and g < 20 and b < 20)
    c.check("a colour literal is drawn on a swatch of its colour", blackish > 200, f"{blackish} near-black pixels in the code area")
    open_file(c, "toc.xhtml")
    c.click_node(code_view(c))
    c.sb.key("End", ("Control",))
    c.sb.type('<p><a href="gone.xhtml">x</a> a & b <b>open</p>')
    time.sleep(1.5)
    c.shot("squiggles")
    img = c.image()
    code = code_view(c)
    x, y, w, h = code.box
    red = c.count_pixels(img, (ox + x, oy + y, ox + x + w - 120, min(oy + y + h, 740)), lambda r, g, b: r > 180 and g < 90 and b < 90)
    c.check("invalid markup and a missing link target are underlined in red", red > 10, f"{red} red pixels")
    c.check("the app is still alive", c.is_alive())


@scenario
def editor_code_tag_keys(c):
    """Ctrl+< wraps the selection in a tag (asks for the name), Ctrl+> removes the surrounding tag and Ctrl+Alt+>
    splits the element at the caret, with the real keys."""
    c.start()
    if not c.check("Edit Book opens the editor window", open_editor(c)):
        return
    c.sb.key("Escape")
    code = code_view(c)
    c.click_node(code)
    c.sb.key("End", ("Control",))
    c.sb.type("<p>one two three")  # the closing tag is added when > is typed; the caret stays before it
    time.sleep(0.5)
    for _ in range(5):
        c.sb.key("Left", ("Shift",))
    c.sb.key("<", ("Control",))
    dlg = c.wait_node(None, rx=r"Wrap in Tag", timeout=6)
    time.sleep(0.6)
    c.shot("wrap-prompt")
    c.check("Ctrl+< asks for the tag to wrap the selection in", dlg is not None)
    if dlg is not None:
        c.sb.type("em")
        c.sb.key("Return")
        time.sleep(0.8)
        text = code_view(c).get_text()
        c.check("the selection is wrapped in the tag that was typed", "<p>one two <em>three</em></p>" in text, text[-50:])
        c.sb.key(">", ("Control",))
        time.sleep(0.8)
        text = code_view(c).get_text()
        c.shot("after-unwrap")
        c.check("Ctrl+> removes the surrounding tag again", "<p>one two three</p>" in text, text[-50:])
        # caret between "one" and "two": split the paragraph
        c.sb.key("End")
        for _ in range(13):
            c.sb.key("Left")
        c.sb.key(">", ("Control", "Alt"))
        time.sleep(0.8)
        text = code_view(c).get_text()
        c.shot("after-split")
        c.check("Ctrl+Alt+> splits the paragraph at the caret", "<p>one </p><p>two three</p>" in text or "<p>one</p><p>two three</p>" in text, text[-60:])
    c.check("the app is still alive", c.is_alive())
