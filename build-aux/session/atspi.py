"""AT-SPI view of the app under test: the accessibility tree is the oracle for "is this widget
there, is it showing, what does it say" and gives the click positions for the real pointer."""
import re
import time

import gi

gi.require_version("Atspi", "2.0")
from gi.repository import Atspi  # noqa: E402

# GTK 4 reports widget positions relative to the window, not the screen, on Wayland. The window
# content starts below the shell's top bar (32 px). Measured against screenshots, GTK4's numbers
# are a constant (7, 5) px left/above of what is drawn, so clicks use this origin.
ORIGIN = (7, 37)


class Node:
    def __init__(self, acc, path, depth):
        self.acc = acc
        self.role = acc.get_role_name()
        self.name = acc.get_name() or ""
        ss = acc.get_state_set()
        self.states = {s for s in range(0, 45) if ss.contains(Atspi.StateType(s))}
        self.showing = ss.contains(Atspi.StateType.SHOWING)
        self.focused = ss.contains(Atspi.StateType.FOCUSED)
        self.checked = ss.contains(Atspi.StateType.CHECKED)
        self.sensitive = ss.contains(Atspi.StateType.SENSITIVE)
        self.path = path
        self.depth = depth
        try:
            e = acc.get_component_iface().get_extents(Atspi.CoordType.WINDOW)
            self.box = (e.x, e.y, e.width, e.height)
        except Exception:
            self.box = (0, 0, 0, 0)
        self.text = None

    def center(self, origin=ORIGIN):
        x, y, w, h = self.box
        return (origin[0] + x + w / 2, origin[1] + y + h / 2)

    def get_text(self):
        try:
            t = self.acc.get_text_iface()
            return Atspi.Text.get_text(t, 0, Atspi.Text.get_character_count(t)) if t else ""
        except Exception:
            return ""

    def actions(self):
        try:
            a = self.acc.get_action_iface()
            return [a.get_action_name(i) for i in range(a.get_n_actions())] if a else []
        except Exception:
            return []

    def __repr__(self):
        return f"<{self.role} {self.name!r} {self.box}{'' if self.showing else ' hidden'}>"


class Tree:
    """A snapshot of every *showing* node of one application."""

    def __init__(self, app_name="bookx", include_hidden=False):
        self.nodes = []
        desk = Atspi.get_desktop(0)
        for i in range(desk.get_child_count()):
            a = desk.get_child_at_index(i)
            if a and (a.get_name() or "").lower() == app_name:
                self._walk(a, (), 0, include_hidden)

    def _walk(self, acc, path, depth, hidden):
        try:
            n = Node(acc, path, depth)
        except Exception:
            return
        if depth > 0:
            self.nodes.append(n)
        if not n.showing and depth > 0 and not hidden:
            return
        try:
            count = acc.get_child_count()
        except Exception:
            return
        for i in range(count):
            c = acc.get_child_at_index(i)
            if c is not None:
                self._walk(c, path + (i,), depth + 1, hidden)

    def find(self, role=None, name=None, rx=None, only_showing=True):
        out = []
        for n in self.nodes:
            if only_showing and not n.showing:
                continue
            if role and n.role != role and not (isinstance(role, (list, tuple)) and n.role in role):
                continue
            if name is not None and n.name != name:
                continue
            if rx is not None and not re.search(rx, n.name):
                continue
            out.append(n)
        return out

    def dump(self):
        return "\n".join(f"{'  ' * n.depth}{n.role} {n.name!r} {n.box}{'' if n.showing else ' (hidden)'}"
                         for n in self.nodes)


def wait_for(fn, timeout=10.0, every=0.25):
    t0 = time.time()
    last = None
    while time.time() - t0 < timeout:
        try:
            last = fn()
        except Exception as e:  # the tree changes under us
            last = None
            err = e  # noqa: F841
        if last:
            return last
        time.sleep(every)
    return last
